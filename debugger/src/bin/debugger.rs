use anyhow::{anyhow, Context, Result};
use probe_rs::{
    architecture::arm::component::{Dwt, Itm, enable_tracing},
    architecture::arm::swo::SwoConfig,
    flashing::{self, Format},
    Core, MemoryInterface, Probe,
};
use probe_rs_cli_util;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

fn main() -> Result<()> {
    // Get a list of all available debug probes
    let probes = Probe::list_all();
    if probes.is_empty() {
        return Err(anyhow!("No probes available"));
    }
    println!("Found {} probe(s): {:#?}", probes.len(), probes);

    // Use the first probe found
    println!("Opening the first probe...");
    let probe = probes[0].open()?;

    // Attach to a chip
    println!("Attaching under reset...");
    let mut session = probe.attach_under_reset("stm32f4")?;
    let comp = session.get_arm_component()?;

    session.setup_swv(
        &SwoConfig::new(16_000_000)
            .set_baud(2_000_000)
            .set_continuous_formatting(false),
    )?;

    // Select a core
    let mut core = session.core(0)?;

    // Halt the attached core
    core.halt(Duration::from_secs(5))?;
    assert!(core.core_halted()?);

    enable_tracing(&mut core)?;

    Dwt::new(&mut core, &comp).enable_exception_trace()?;
    const DWT_CTRL_ADDR: u32 = 0xE0001000;
    let mut ctrl: u32 = core.read_word_32(DWT_CTRL_ADDR)?;
    ctrl &= !(1 << 12); // clear PCSAMLENA; not handled properly by itm-tools encoder
    ensure_write_word_32(&mut core, DWT_CTRL_ADDR, ctrl)?;

    let mut itm = Itm::new(&mut core, &comp);
    itm.unlock()?;
    itm.tx_enable()?;

    core.run()?;
    drop(core);

    println!("Flashing application...");
    flash_program(&mut session)?;

    let mut f = File::create("./itm.bin")?;
    println!("Recording all ITM packets to {:#?}", f);
    while let Ok(bytes) = session.read_swo() {
        if bytes.len() > 0 {
            f.write_all(&bytes)?;
        }

        if session.core(0)?.core_halted()? {
            break;
        }
    }

    println!("Done.");
    Ok(())
}

fn ensure_write_word_32(core: &mut Core, addr: u32, val: u32) -> Result<()> {
    core.write_word_32(addr, val)?;
    if core.read_word_32(addr)? != val {
        return Err(anyhow!("readback of register {} is unexpected!"));
    }

    Ok(())
}

fn flash_program(session: &mut probe_rs::Session) -> Result<()> {
    let work_dir = PathBuf::from("./application");
    // XXX always debug
    let path = probe_rs_cli_util::build_artifact(
        &work_dir,
        &["--bin".to_string(), "application".to_string()],
    )?;
    flashing::download_file(session, &path, Format::Elf).context("failed to flash target")
}

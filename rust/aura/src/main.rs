use aura::flags::{SubCmd, ToArgs};
use aura_arch as arch;
use clap::Clap;
use std::process::Command;

#[derive(Debug)]
enum Error {
    IO(std::io::Error),
    Arch(arch::Error),
}

fn main() -> Result<(), Error> {
    let args = aura::flags::Args::parse();
    // let raw = aura::flags::Args::into_app().get_matches();
    let alpm = arch::open_alpm().map_err(Error::Arch)?;

    match args.subcmd {
        // --- Pacman Commands --- //
        SubCmd::Database(_) => unimplemented!(),
        SubCmd::Files(_) => unimplemented!(),
        SubCmd::Query(_) => unimplemented!(),
        SubCmd::Remove(_) => unimplemented!(),
        SubCmd::DepTest(_) => unimplemented!(),
        SubCmd::Upgrade(_) => unimplemented!(),
        SubCmd::Sync(s) => {
            let back = s.to_args();
            Command::new("pacman")
                .args(back)
                .status()
                .map_err(Error::IO)?;
        }
        // --- AUR Packages --- //
        SubCmd::AurSync => unimplemented!(),
        SubCmd::Backup => unimplemented!(),
        SubCmd::Cache => unimplemented!(),
        SubCmd::Log => unimplemented!(),
        SubCmd::Languages => unimplemented!(),
        SubCmd::ViewConf => unimplemented!(),
        SubCmd::Extra => unimplemented!(),
        // --- Orphan Packages --- //
        SubCmd::Orphans(o) if o.abandon => unimplemented!(),
        SubCmd::Orphans(o) if !o.adopt.is_empty() => unimplemented!(),
        SubCmd::Orphans(_) => arch::orphans(&alpm)
            .iter()
            .for_each(|o| println!("{}", o.name())),
        // --- PKGBUILD Analysis --- //
        SubCmd::Analysis(_) => unimplemented!(),
    }

    Ok(())
}
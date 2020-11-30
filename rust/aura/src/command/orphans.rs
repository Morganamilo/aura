//! All functionality involving the `-O` command.

use crate::{a, aln};
use alpm::{Alpm, PackageReason, TransFlag};
use anyhow::{ensure, Context, Result};
use aura_arch as arch;
use colored::*;
use i18n_embed::fluent::FluentLanguageLoader;
use i18n_embed_fl::fl;
use std::collections::HashSet;
use ubyte::ToByteUnit;

/// Print the name of each orphaned package.
pub fn list(alpm: &Alpm) {
    arch::orphans(&alpm)
        .iter()
        .for_each(|o| println!("{}", o.name()))
}

/// Sets a package's install reason to "as explicit". An alias for `-D --asexplicit`.
pub fn adopt(alpm: &Alpm, fll: FluentLanguageLoader, packages: Vec<String>) -> Result<()> {
    let db = alpm.localdb();
    let reals: Vec<_> = packages
        .into_iter()
        .filter_map(|p| db.pkg(p).ok())
        .collect();

    ensure!(!reals.is_empty(), "packages are not installed");
    for mut p in reals {
        p.set_reason(PackageReason::Explicit)
            .context("failed to set packages as explicit")?;
        let msg = format!("{}", fl!(fll, "orphans-adopt", package = p.name()).green());
        aln!(&msg);
    }

    Ok(())
}

/// Uninstall all orphan packages.
///
/// Will fail if the process does not have permission to create the lockfile,
/// which usually lives in a root-owned directory.
pub fn remove(alpm: &mut Alpm, fll: FluentLanguageLoader) -> Result<()> {
    // Check for orphans.
    let orphans = arch::orphans(alpm);
    if !orphans.is_empty() {
        // Copy the name of each original orphan.
        let names: HashSet<_> = orphans.iter().map(|p| p.name().to_string()).collect();

        // Initialize the transaction.
        let mut flag = TransFlag::RECURSE;
        flag.insert(TransFlag::UNNEEDED);
        alpm.trans_init(flag)
            .context("failed to initiate transaction")?;

        for p in orphans {
            alpm.trans_remove_pkg(p)?;
        }

        // Advance the transaction, calculating the effects of the TransFlags.
        alpm.trans_prepare()
            .map_err(|(_, e)| e)
            .context("failed to prepare transaction")?;

        // Notify the user of the results.
        let removal = alpm.trans_remove();
        let longest = removal.iter().map(|p| p.name().len()).max().unwrap_or(0);
        aln!(format!("{}\n", fl!(fll, "orphans-abandon").yellow()));
        for p in removal {
            let size = format!("{}", p.isize().bytes());
            if names.contains(p.name()) {
                print!("  {:w$} ", p.name().cyan(), w = longest);
                println!("{:>9}", size);
            } else {
                println!("  {:w$} {:>9}", p.name(), size, w = longest);
            }
        }
        println!("  {:-<w$}", "-".magenta(), w = longest + 10);
        let total: i64 = removal.iter().map(|p| p.isize()).sum();
        let size = format!("{}", total.bytes());
        println!("  {:w$} {:>9}\n", "Total", size, w = longest);

        // Proceed with the removal if the user accepts.
        let msg = format!("{} {} ", fl!(fll, "proceed"), fl!(fll, "proceed-yes"));
        crate::utils::prompt(&a!(msg))?;
        alpm.trans_commit()
            .map_err(|(_, e)| e)
            .context("failed to commit transaction")?;
        alpm.trans_release()
            .context("failed to release transaction")?;
        aln!("Done.".green());
    }

    Ok(())
}

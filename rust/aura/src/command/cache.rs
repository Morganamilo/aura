//! All functionality involving the `-C` command.

use crate::error::Error;
use crate::{a, aln, aura, green, red, yellow};
use alpm::Alpm;
use aura_arch as arch;
use aura_core as core;
use chrono::{DateTime, Local};
use colored::*;
use i18n_embed::fluent::FluentLanguageLoader;
use i18n_embed_fl::fl;
use itertools::Itertools;
use log::debug;
use pbr::ProgressBar;
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, path::Path};
use ubyte::ToByteUnit;

const FIFTY_MB: i64 = 52_428_800;

const FIVE_HUNDRED_MB: i64 = 524_288_000;

/// Print the contents of the package cache.
pub(crate) fn list(path: &Path) -> Result<(), Error> {
    for de in path.read_dir()?.filter_map(|de| de.ok()) {
        println!("{}", de.path().display());
    }

    Ok(())
}

/// Print cache data for given packages.
pub(crate) fn info(
    fll: FluentLanguageLoader,
    alpm: &Alpm,
    path: &Path,
    packages: Vec<String>,
) -> Result<(), Error> {
    let db = alpm.localdb();

    packages
        .iter()
        .filter_map(|p| core::cache::info(path, p).ok())
        .filter_map(|ci| ci)
        .for_each(|ci| {
            let name = fl!(fll, "common-name");
            let ver = fl!(fll, "cache-info-latest");
            let created = fl!(fll, "cache-info-created");
            let sig = fl!(fll, "cache-info-sig");
            let size = fl!(fll, "cache-info-size");
            let av = fl!(fll, "cache-info-avail");
            let long = vec![&name, &ver, &created, &sig, &size, &av]
                .iter()
                .map(|s| s.len())
                .max()
                .unwrap();

            let dt = DateTime::<Local>::from(ci.created).format("%F %T");
            let is_in = if let Ok(pkg) = db.pkg(ci.name.as_str()) {
                if ci.version == pkg.version().as_str() {
                    format!("[{}]", fl!(fll, "cache-info-installed"))
                        .cyan()
                        .bold()
                } else {
                    format!("[{}: {}]", fl!(fll, "cache-info-installed"), pkg.version())
                        .yellow()
                        .bold()
                }
            } else {
                "".normal()
            };
            let sig_yes_no = if ci.signature {
                fl!(fll, "common-yes").green().bold()
            } else {
                fl!(fll, "common-no").yellow()
            };

            // TODO Handle non-ASCII padding.
            println!("{:w$} : {}", name.bold(), ci.name, w = long);
            println!("{:w$} : {} {}", ver.bold(), ci.version, is_in, w = long);
            println!("{:w$} : {}", created.bold(), dt, w = long);
            println!("{:w$} : {}", sig.bold(), sig_yes_no, w = long);
            println!("{:w$} : {}", size.bold(), ci.size.bytes(), w = long);
            println!("{:w$} : {}", av.bold(), ci.available.join(", "), w = long);
            println!();
        });

    Ok(())
}

/// Print all package filepaths from the cache that match some search term.
pub(crate) fn search(path: &Path, term: &str) -> Result<(), Error> {
    let matches = core::cache::search(path, term)?;
    for file in matches {
        println!("{}", file.path().display());
    }
    Ok(())
}

/// Delete all but `keep`-many old tarballs for each package in the cache.
pub(crate) fn clean(fll: FluentLanguageLoader, path: &Path, keep: usize) -> Result<(), Error> {
    sudo::escalate_if_needed()?;

    let size_before = core::cache::size(path)?;
    let human = format!("{}", size_before.bytes.bytes());
    aura!(fll, "cache-size", size = human);
    yellow!(fll, "cache-clean-keep", pkgs = keep);

    // Proceed if the user accepts.
    let msg = format!("{} {} ", fl!(fll, "proceed"), fl!(fll, "proceed-yes"));
    crate::utils::prompt(&a!(msg))?;

    // Get all the tarball paths, sort and group them by name, and then remove them.
    path.read_dir()?
        .filter_map(|de| de.ok())
        .filter_map(|de| core::cache::PkgPath::new(de.path()))
        .sorted_by(|p0, p1| p1.cmp(&p0)) // Forces a `collect` underneath.
        .group_by(|pp| pp.to_package().name.clone()) // TODO Naughty clone.
        .into_iter()
        .map(|(_, group)| group.skip(keep)) // Thanks to the reverse-sort above, `group` is already backwards.
        .flatten()
        .for_each(|pp| {
            let _ = pp.remove(); // TODO Handle this error better?
        });

    let size_after = core::cache::size(path)?;
    let freed = format!("{}", (size_before.bytes - size_after.bytes).bytes());
    green!(fll, "cache-clean-freed", bytes = freed);
    Ok(())
}

/// Download tarballs of installed packages that are missing from the cache.
pub(crate) fn refresh(fll: FluentLanguageLoader, alpm: &Alpm, path: &Path) -> Result<(), Error> {
    sudo::escalate_if_needed()?;

    // Every version of every package in the cache.
    let groups: HashMap<String, Vec<String>> = path
        .read_dir()?
        .filter_map(|de| de.ok())
        .filter_map(|de| core::cache::PkgPath::new(de.path()))
        .map(|pp| {
            let p = core::common::Package::from(pp);
            (p.name, p.version)
        })
        .into_group_map();

    // TODO Getting the packages from the sync database isn't quite right. It
    // won't pull the versions of things currently installed, it'll pull
    // whatever it thinks the newest is.
    let ps: Vec<_> = arch::officials(alpm)
        .filter(|p| {
            let pv = p.version().as_str();
            groups
                .get(p.name())
                .map(|vs| !vs.iter().any(|v| v == pv))
                .unwrap_or(true)
        })
        .sorted_by(|a, b| a.name().cmp(b.name()))
        .collect();

    if ps.is_empty() {
        green!(fll, "cache-refresh-no-work");
    } else {
        let long_n = ps.iter().map(|p| p.name().chars().count()).max().unwrap();
        let long_v = ps
            .iter()
            .map(|p| p.version().as_str().chars().count())
            .max()
            .unwrap();
        // TODO Localize.
        let p = format!("Package ({})", ps.len()).bold();
        let v = "Version".bold();
        let s = "Download Size";
        let total = colour_size(ps.iter().map(|p| p.download_size()).sum());
        let span = long_n + long_v;

        // Display a summary.
        println!("{:n$} {:v$} {}\n", p, v, s.bold(), n = long_n, v = long_v);
        for p in ps.iter() {
            let n = p.name();
            let v = p.version().as_str();
            let s = colour_size(p.download_size());
            println!("{:n$} {:v$} {:>9}", n, v, s, n = long_n, v = long_v);
        }
        println!("{:-<w$}", "-".magenta(), w = span + s.chars().count());
        println!(
            "{:w$} {:>9}\n",
            fl!(fll, "common-total").bold(),
            total,
            w = span + 1
        );

        // Proceed if the user accepts.
        let msg = format!("{} {} ", fl!(fll, "proceed"), fl!(fll, "proceed-yes"));
        crate::utils::prompt(&a!(msg))?;

        // Mirrors.
        let mirrors: HashMap<&str, Vec<&str>> = alpm
            .syncdbs()
            .iter()
            .map(|db| (db.name(), db.servers().into_iter().collect()))
            .collect();

        // Syncable package values.
        let foo: Vec<_> = ps
            .iter()
            .filter_map(|p| match (p.db(), p.arch()) {
                (Some(db), Some(arch)) => mirrors
                    .get(db.name())
                    .map(|ms| (p.name(), p.version().as_str(), arch, p.download_size(), ms)),
                _ => None,
            })
            .collect();

        // let mb = Arc::new(MultiBar::new());
        // std::thread::spawn({
        //     let mb = Arc::clone(&mb);
        //     move || {
        //         mb.listen();
        //     }
        // });

        foo.into_par_iter().for_each(|(n, v, a, _, ms)| {
            let tarball = format!("{}-{}-{}.pkg.tar.zst", n, v, a);
            let url = format!("{}/{}", ms[0], tarball); // TODO dangerous
            let mut target = path.to_path_buf();
            target.push(tarball);
            // let mut bar = mb.create_bar(bytes as u64);
            // bar.set_units(Units::Bytes);
            // bar.message(&format!("{}-{} ", n, v));
            println!("Downloading: {}", n);
            let _ = crate::download::download(&url, &target);
        });

        // mb.listen();

        // if let Some((n, v, a, bytes, ms)) = foo.first() {
        //     let tarball = format!("{}-{}-{}.pkg.tar.zst", n, v, a);
        //     let url = format!("{}/{}", ms[0], tarball);
        //     let mut target = path.to_path_buf();
        //     target.push(tarball);
        //     let mut bar = ProgressBar::new(*bytes as u64);
        //     bar.set_units(Units::Bytes);
        //     bar.message(&format!("{}-{} ", n, v));
        //     crate::download::download_with_progress(&url, &target, Some(bar))?;
        // }

        green!(fll, "common-done");
    }

    Ok(())
}

/// Colour a size string depending on the count of bytes.
fn colour_size(size: i64) -> ColoredString {
    if size >= FIVE_HUNDRED_MB {
        size.bytes().to_string().red()
    } else if size >= FIFTY_MB {
        size.bytes().to_string().yellow()
    } else {
        size.bytes().to_string().normal()
    }
}

/// Backup the package cache to a given directory.
pub(crate) fn backup(fll: FluentLanguageLoader, source: &Path, target: &Path) -> Result<(), Error> {
    // The full, absolute path to copy files to.
    let full: PathBuf = if target.is_absolute() {
        target.to_path_buf()
    } else {
        let mut curr = std::env::current_dir()?;
        curr.push(target);
        curr
    };
    let ts = full.to_str().unwrap();

    // Exit early if the target is an existing file, not a directory.
    if target.is_file() {
        red!(fll, "cache-backup-file", target = ts);
        Err(Error::Silent)?;
    }

    // How big is the current cache?
    let cache_size: core::cache::CacheSize = core::cache::size(source)?;
    let size = format!("{}", cache_size.bytes.bytes());
    aura!(fll, "cache-size", size = size);

    // Is the target directory empty?
    let target_count = target.read_dir().map(|d| d.count()).unwrap_or(0);
    if target_count > 0 {
        yellow!(fll, "cache-backup-nonempty", target = ts);
    } else {
        aura!(fll, "cache-backup-target", target = ts);
    }

    // Proceed if the user accepts.
    let msg = format!("{} {} ", fl!(fll, "proceed"), fl!(fll, "proceed-yes"));
    crate::utils::prompt(&a!(msg))?;
    copy(source, &full, cache_size.files)
}

/// Copy all the cache files concurrently.
fn copy(source: &Path, target: &Path, file_count: u64) -> Result<(), Error> {
    debug!("Begin cache copying.");

    // TODO Change the bar style.
    // A progress bar to display the copying progress.
    let pb = Arc::new(Mutex::new(ProgressBar::new(file_count)));

    // Silently succeeds if the directory already exists.
    std::fs::create_dir_all(target)?;

    source
        .read_dir()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let from = entry.path();
            entry.path().file_name().map(|name| {
                let mut to = target.to_path_buf();
                to.push(name);
                (from, to)
            })
        })
        .par_bridge()
        .for_each(|(from, to)| {
            if std::fs::copy(from, to).is_ok() {
                pb.lock().unwrap().inc();
            }
        });
    Ok(())
}

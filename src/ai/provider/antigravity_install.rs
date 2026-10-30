//! Managed runtime installation adapted from T3 Code (MIT).
//! Pinned Google archive, exact sizes/SHA-256, staged pair, native handshake,
//! then atomic activation. No credentials are downloaded or imported.
use super::{acp, agent_program, resolve_program};
use crate::{ai::AntigravityEvent, config::AiSettings};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, sync::mpsc::UnboundedSender};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Member {
    name: String,
    bytes: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Release {
    version: String,
    url: String,
    sha256: String,
    archive_bytes: u64,
    executable: Member,
    harness: Member,
}

fn release() -> Result<Release> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    let catalog: BTreeMap<String, Release> =
        serde_json::from_str(include_str!("antigravity-releases.json"))?;
    catalog
        .get(&format!("{os}-{arch}"))
        .cloned()
        .context("Google does not publish a supported Antigravity runtime for this platform")
}
fn root() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|root| root.join("gitside/runtimes/antigravity"))
        .context("Could not locate the runtime installation directory")
}
fn completed(root: &Path, release: &Release) -> Option<PathBuf> {
    let directory = root.join("versions").join(&release.sha256);
    let record = directory.join(".install-complete.json");
    if fs::metadata(&record).ok()?.len() > 8192 {
        return None;
    }
    let saved: Release = serde_json::from_slice(&fs::read(record).ok()?).ok()?;
    if &saved != release {
        return None;
    }
    for member in [&release.executable, &release.harness] {
        let path = directory.join(&member.name);
        if fs::symlink_metadata(&path).ok()?.file_type().is_symlink()
            || fs::metadata(&path).ok()?.len() != member.bytes
            || resolve_program(path.to_str()?).is_none()
        {
            return None;
        }
    }
    Some(directory.join(&release.executable.name))
}
pub(crate) fn installed() -> Option<PathBuf> {
    let (root, release) = (root().ok()?, release().ok()?);
    let active = root.join("active.json");
    if fs::metadata(&active).ok()?.len() > 8192 {
        return None;
    }
    let active: serde_json::Value = serde_json::from_slice(&fs::read(active).ok()?).ok()?;
    if active["releaseId"] != release.sha256 {
        return None;
    }
    completed(&root, &release)
}
fn report(events: Option<&UnboundedSender<AntigravityEvent>>, message: String) {
    if let Some(events) = events {
        let _ = events.send(AntigravityEvent::Status(message));
    } else {
        eprintln!("{message}");
    }
}
fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn activate(root: &Path, release: &Release) -> Result<()> {
    let mut pointer = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer(
        pointer.as_file_mut(),
        &serde_json::json!({"releaseId":release.sha256}),
    )?;
    pointer.as_file().sync_all()?;
    pointer
        .persist(root.join("active.json"))
        .map_err(|error| error.error)?;
    Ok(())
}

/// Honor explicit/PATH installations; otherwise install and use the managed pair.
pub(crate) async fn ensure(
    settings: &AiSettings,
    events: Option<&UnboundedSender<AntigravityEvent>>,
) -> Result<PathBuf> {
    if let Some(path) = resolve_program(&agent_program(settings)) {
        let harness = path
            .parent()
            .context("Runtime has no parent directory")?
            .join(if cfg!(windows) {
                "localharness_external.exe"
            } else {
                "localharness_external"
            });
        if resolve_program(harness.to_str().context("Runtime path is not UTF-8")?).is_none() {
            bail!(
                "Antigravity's localharness_external helper is missing or not executable; install the complete matching runtime archive"
            );
        }
        return Ok(path);
    }
    if settings
        .agent
        .command
        .as_deref()
        .is_some_and(|command| !command.trim().is_empty())
    {
        bail!(
            "The configured Antigravity executable was not found; clear its path to use managed installation"
        );
    }
    install(events).await
}

pub(crate) async fn install(events: Option<&UnboundedSender<AntigravityEvent>>) -> Result<PathBuf> {
    let (root, release) = (root()?, release()?);
    private_directory(&root)?;
    private_directory(&root.join("versions"))?;
    if let Some(path) = completed(&root, &release) {
        activate(&root, &release)?;
        return Ok(path);
    }
    let url = reqwest::Url::parse(&release.url)?;
    if url.scheme() != "https"
        || url.host_str() != Some("dl.google.com")
        || !url.path().starts_with("/agy-extensions/releases/")
    {
        bail!("Invalid pinned Google runtime URL");
    }
    report(
        events,
        format!("Downloading Antigravity {}…", release.version),
    );
    let stage = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(&root)?;
    let archive = stage.path().join("download.zip");
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(900))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("gitside/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut response = client
        .get(url)
        .send()
        .await
        .context("Could not download the Antigravity runtime")?;
    if !response.status().is_success() {
        bail!("Google runtime download returned {}", response.status());
    }
    if response
        .content_length()
        .is_some_and(|size| size != release.archive_bytes)
    {
        bail!("Google runtime download size does not match the pinned release");
    }
    let mut output = tokio::fs::File::create(&archive).await?;
    let mut downloaded = 0u64;
    let mut hash = Sha256::new();
    let mut last = Instant::now();
    while let Some(chunk) = response.chunk().await? {
        downloaded += chunk.len() as u64;
        if downloaded > release.archive_bytes {
            bail!("Antigravity download exceeded its pinned size");
        }
        hash.update(&chunk);
        output.write_all(&chunk).await?;
        if events.is_some() && last.elapsed() >= Duration::from_millis(250) {
            report(
                events,
                format!(
                    "Downloading Antigravity: {} / {} MiB",
                    downloaded / 1_048_576,
                    release.archive_bytes / 1_048_576
                ),
            );
            last = Instant::now();
        }
    }
    output.sync_all().await?;
    drop(output);
    check_download(&release, downloaded, &format!("{:x}", hash.finalize()))?;
    report(events, "Extracting the verified runtime…".into());
    let pinned = release.clone();
    // The blocking task owns the staging directory, so cancellation cannot remove
    // files while it is extracting or publish a half-finished installation.
    let stage = tokio::task::spawn_blocking(move || extract(stage, &pinned))
        .await
        .context("Runtime extraction task failed")??;
    let pair = stage.path().join("pair");
    report(events, "Verifying Antigravity startup…".into());
    acp::verify_runtime(&pair.join(&release.executable.name), &release.version).await?;
    fs::write(
        pair.join(".install-complete.json"),
        serde_json::to_vec(&release)?,
    )?;
    let destination = root.join("versions").join(&release.sha256);
    match fs::rename(&pair, &destination) {
        Ok(()) => {}
        Err(_) if completed(&root, &release).is_some() => {} // Another installer finished first.
        Err(error) => {
            return Err(error).context(
                "Could not publish the verified runtime; existing installations were kept",
            );
        }
    }
    activate(&root, &release)?;
    report(events, format!("Antigravity {} installed", release.version));
    completed(&root, &release).context("Installed runtime could not be verified")
}
fn check_download(release: &Release, bytes: u64, hash: &str) -> Result<()> {
    if bytes != release.archive_bytes || hash != release.sha256 {
        bail!("Antigravity archive failed its size or SHA-256 check; nothing was installed");
    }
    Ok(())
}
fn extract(stage: TempDir, release: &Release) -> Result<TempDir> {
    let mut archive = zip::ZipArchive::new(fs::File::open(stage.path().join("download.zip"))?)?;
    if archive.len() != 2 {
        bail!("Runtime archive must contain exactly the executable and its harness");
    }
    let pair = stage.path().join("pair");
    private_directory(&pair)?;
    let mut seen = Vec::new();
    for index in 0..archive.len() {
        let mut member = archive.by_index(index)?;
        let expected = [&release.executable, &release.harness]
            .into_iter()
            .find(|expected| expected.name == member.name())
            .context("Runtime archive contains an unexpected or unsafe path")?;
        if seen.contains(&expected.name)
            || member.is_dir()
            || member.size() != expected.bytes
            || member
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
        {
            bail!("Runtime archive contains an unsafe, duplicate or incorrectly sized member");
        }
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(pair.join(&expected.name))?;
        let copied = std::io::copy(&mut (&mut member).take(expected.bytes + 1), &mut output)?;
        if copied != expected.bytes {
            bail!("Runtime archive member was truncated or oversized");
        }
        output.flush()?;
        output.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(pair.join(&expected.name), fs::Permissions::from_mode(0o700))?;
        }
        seen.push(expected.name.clone());
    }
    Ok(stage)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(names: &[&str]) -> (TempDir, Release) {
        let stage = tempfile::tempdir().unwrap();
        let file = fs::File::create(stage.path().join("download.zip")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for name in names {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default().unix_permissions(0o700),
            )
            .unwrap();
            zip.write_all(b"test").unwrap();
        }
        zip.finish().unwrap();
        let data = fs::read(stage.path().join("download.zip")).unwrap();
        let release = Release {
            version: "test".into(),
            url: "https://dl.google.com/agy-extensions/test.zip".into(),
            sha256: format!("{:x}", Sha256::digest(&data)),
            archive_bytes: data.len() as u64,
            executable: Member {
                name: "server".into(),
                bytes: 4,
            },
            harness: Member {
                name: "harness".into(),
                bytes: 4,
            },
        };
        (stage, release)
    }
    #[test]
    fn rejects_corrupt_truncated_and_unexpected_archives_without_activation() {
        let (stage, release) = fixture(&["server", "harness"]);
        check_download(&release, release.archive_bytes, &release.sha256).unwrap();
        assert!(check_download(&release, release.archive_bytes - 1, &release.sha256).is_err());
        assert!(check_download(&release, release.archive_bytes, "incorrect").is_err());
        for names in [
            &["../server", "harness"][..],
            &["server"][..],
            &["server", "harness", "extra"][..],
        ] {
            let (stage, release) = fixture(names);
            assert!(extract(stage, &release).is_err());
        }
        let (duplicate, pinned) = fixture(&["server", "serve2"]);
        let path = duplicate.path().join("download.zip");
        let bytes = fs::read(&path).unwrap();
        let bytes = bytes
            .windows(6)
            .enumerate()
            .filter_map(|(index, window)| (window == b"serve2").then_some(index))
            .collect::<Vec<_>>();
        let mut data = fs::read(&path).unwrap();
        for index in bytes {
            data[index..index + 6].copy_from_slice(b"server");
        }
        fs::write(&path, data).unwrap();
        assert!(extract(duplicate, &pinned).is_err());
        let mut wrong_size = release;
        wrong_size.harness.bytes += 1;
        assert!(extract(stage, &wrong_size).is_err());
    }
    #[test]
    fn publishes_only_completed_pairs_and_keeps_activation_atomic() {
        let root = tempfile::tempdir().unwrap();
        private_directory(&root.path().join("versions")).unwrap();
        let (stage, release) = fixture(&["server", "harness"]);
        let stage = extract(stage, &release).unwrap();
        let destination = root.path().join("versions").join(&release.sha256);
        fs::rename(stage.path().join("pair"), &destination).unwrap();
        assert!(completed(root.path(), &release).is_none());
        fs::write(
            destination.join(".install-complete.json"),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();
        assert!(completed(root.path(), &release).is_some());
        activate(root.path(), &release).unwrap();
        let old = fs::read(root.path().join("active.json")).unwrap();
        activate(root.path(), &release).unwrap();
        assert_eq!(old, fs::read(root.path().join("active.json")).unwrap());
        fs::remove_file(destination.join("harness")).unwrap();
        assert!(completed(root.path(), &release).is_none());
    }
}

//! Mac arm64 and Windows x64 packaging. Personal configuration is never included.
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
};
pub fn run(args: &[String]) -> Result<()> {
    let version = args.first().map(String::as_str).unwrap_or("0.14.0-api.20261030.1");
    if args.len() > 1 || version.is_empty() || !version.bytes().all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)) {
        bail!("expected one safe package version");
    }
    if cfg!(target_os = "windows") {
        return windows_package(version);
    }
    if !cfg!(target_os = "macos") {
        bail!("local-api-package requires macOS or Windows");
    }
    let root = crate::gates::root();
    let target = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("target"));
    let binary = std::env::var_os("PDFCRAFT_PACKAGE_BINARY").map(std::path::PathBuf::from).unwrap_or_else(|| target.join("release/pdfcraft"));
    if !binary.is_file() {
        bail!("build cargo build --release -p pdfcraft first");
    }
    let architectures = Command::new("/usr/bin/lipo").arg("-archs").arg(&binary).output()?;
    if !architectures.status.success() || String::from_utf8_lossy(&architectures.stdout).trim() != "arm64" {
        bail!("the macos-arm64 package requires an arm64 Mach-O executable");
    }
    let output = root.join("dist");
    fs::create_dir_all(&output)?;
    let archive = output.join(format!("PdfCraft-{version}-macos-arm64.zip"));
    if archive.exists() {
        bail!("refusing to overwrite {}", archive.display());
    }
    // Keep the transient .app out of indexed/synced Documents folders. Such
    // watchers can reattach Finder metadata between xattr cleanup and signing.
    let staging = std::env::temp_dir().join(format!("pdfcraft-local-api-package-{}", std::process::id()));
    // create_dir refuses an existing name: cleanup below owns only this newly created tree.
    fs::create_dir(&staging)?;
    let result = (|| -> Result<()> {
        let app = staging.join("PdfCraft.app");
        let contents = app.join("Contents");
        let exe = contents.join("MacOS/PdfCraft");
        let resources = contents.join("Resources");
        fs::create_dir_all(exe.parent().context("executable directory")?)?;
        fs::create_dir(&resources)?;
        fs::copy(binary, &exe)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o755))?;
        }
        fs::copy(root.join("assets/app-icon/pdfcraft.icns"), resources.join("PdfCraft.icns"))?;
        let commit = Command::new("git").current_dir(&root).args(["rev-parse", "HEAD"]).output()?;
        if !commit.status.success() {
            bail!("could not determine source commit");
        }
        let sha = String::from_utf8(commit.stdout)?;
        let template = fs::read_to_string(root.join("packaging/macos/Info.plist.in"))?;
        let info = template.replace("@SHORT_VERSION@", "0.14.0").replace("@VERSION@", version).replace("@BUILD_SHA@", sha.trim());
        fs::write(contents.join("Info.plist"), info)?;
        fs::write(contents.join("PkgInfo"), b"APPL????")?;
        populate_resources(&root, &resources)?;
        check(Command::new("/usr/bin/xattr").args(["-cr"]).arg(&app))?;
        check(Command::new("/usr/bin/codesign").args(["--force", "--deep", "--sign", "-"]).arg(&app))?;
        check(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&app))?;
        check(Command::new("/usr/bin/ditto").args(["-c", "-k", "--keepParent", "--norsrc", "--noextattr"]).arg(&app).arg(&archive))?;
        println!("{}", archive.display());
        Ok(())
    })();
    let cleaned = fs::remove_dir_all(&staging);
    result?;
    cleaned?;
    Ok(())
}
fn populate_resources(root: &Path, resources: &Path) -> Result<()> {
    for name in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md"] {
        fs::copy(root.join(name), resources.join(name))?;
    }
    fs::copy(root.join("docs/local-api-translation.md"), resources.join("API翻译使用说明.md"))?;
    fs::copy(root.join("assets/pcb-glossary-public.json"), resources.join("pcb-glossary-public.json"))?;
    let fonts = std::env::var_os("CRAFT_FONTS_DIR").context("CRAFT_FONTS_DIR is required for the Chinese edition")?;
    let fonts = Path::new(&fonts);
    fs::copy(fonts.join("ATTRIBUTION.md"), resources.join("craft-fonts-ATTRIBUTION.md"))?;
    for entry in fs::read_dir(fonts.join("fonts"))? {
        let entry = entry?;
        let license = entry.path().join("OFL.txt");
        if license.is_file() {
            fs::copy(license, resources.join(format!("OFL-{}.txt", entry.file_name().to_string_lossy())))?;
        }
    }
    for entry in fs::read_dir(root.join("assets/fonts"))? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("OFL") {
            fs::copy(entry.path(), resources.join(entry.file_name()))?;
        }
    }
    let models = resources.join("models");
    // Only manifest-named, hash-verified resources may come from an offline cache.
    if let Some(cache) = std::env::var_os("PDFCRAFT_MODELS_DIR") {
        fs::create_dir(&models)?;
        let manifest = crate::assets::load(root)?;
        for model in manifest.fetched.iter().filter(|m| m.kind == "model") {
            for (file, hash) in [(model.file.clone(), model.sha256.as_str()), (format!("{}.LICENCE.txt", model.file), model.licence_sha256.as_str())]
            {
                let path = Path::new(&cache).join(&file);
                let bytes = verified_bytes(fs::File::open(&path).with_context(|| format!("reading {}", path.display()))?, hash, 64 << 20)?;
                fs::write(models.join(file), bytes)?;
            }
        }
    }
    crate::assets::models(&[models.to_string_lossy().into_owned()])?;
    Ok(())
}
fn windows_package(version: &str) -> Result<()> {
    let root = crate::gates::root();
    let target = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("target"));
    let binary = std::env::var_os("PDFCRAFT_PACKAGE_BINARY").map(std::path::PathBuf::from).unwrap_or_else(|| target.join("release/pdfcraft.exe"));
    verify_win64(fs::File::open(&binary).context("build the Windows x64 executable first")?)?;
    let output = root.join("dist");
    fs::create_dir_all(&output)?;
    let archive = output.join(format!("PdfCraft-{version}-win64.zip"));
    if archive.exists() {
        bail!("refusing to overwrite {}", archive.display());
    }
    let staging = std::env::temp_dir().join(format!("pdfcraft-win64-package-{}", std::process::id()));
    fs::create_dir(&staging)?;
    let mut owns_archive = false;
    let result = (|| -> Result<()> {
        let app = staging.join("PdfCraft");
        fs::create_dir(&app)?;
        fs::copy(&binary, app.join("PdfCraft.exe"))?;
        populate_resources(&root, &app)?;
        let commit = Command::new("git").current_dir(&root).args(["rev-parse", "HEAD"]).output()?;
        if !commit.status.success() {
            bail!("could not determine source commit");
        }
        fs::write(
            app.join("VERSION.txt"),
            format!(
                "PdfCraft API Edition {version}\nCommit: {}\nWindows 10/11 x64; extract the entire folder before running PdfCraft.exe.\nAPI settings use the user profile; no keys are included.\n",
                String::from_utf8(commit.stdout)?.trim()
            ),
        )?;
        let file = fs::OpenOptions::new().write(true).create_new(true).open(&archive)?;
        owns_archive = true;
        let mut zip = zip::ZipWriter::new(file);
        zip_tree(&mut zip, &staging, &app)?;
        zip.finish()?.sync_all()?;
        Ok(())
    })();
    let cleaned = fs::remove_dir_all(&staging);
    if result.is_err() && owns_archive {
        fs::remove_file(&archive)?;
    }
    result?;
    cleaned?;
    println!("{}", archive.display());
    Ok(())
}
fn verify_win64(mut reader: impl Read + Seek) -> Result<()> {
    let mut dos = [0_u8; 64];
    reader.read_exact(&mut dos)?;
    if dos.get(..2) != Some(b"MZ") {
        bail!("not a PE executable");
    }
    let offset = u32::from_le_bytes(dos.get(60..64).context("PE offset")?.try_into()?);
    if !(64..=1_048_576).contains(&offset) {
        bail!("invalid PE header offset");
    }
    reader.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut pe = [0_u8; 96];
    reader.read_exact(&mut pe)?;
    if pe.get(..6) != Some(&b"PE\0\0\x64\x86"[..]) || pe.get(24..26) != Some(&b"\x0b\x02"[..]) || pe.get(92..94) != Some(&b"\x02\x00"[..]) {
        bail!("the win64 package requires an x86-64 PE32+ GUI executable");
    }
    Ok(())
}
fn zip_tree<W: std::io::Write + Seek>(zip: &mut zip::ZipWriter<W>, root: &Path, dir: &Path) -> Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            bail!("package staging must not contain symlinks");
        }
        if kind.is_dir() {
            zip_tree(zip, root, &entry.path())?;
        } else if kind.is_file() {
            let path = entry.path();
            let name = path.strip_prefix(root)?.to_str().context("package path is not UTF-8")?.replace('\\', "/");
            zip.start_file(name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored))?;
            std::io::copy(&mut fs::File::open(path)?, zip)?;
        }
    }
    Ok(())
}
fn verified_bytes(reader: impl Read, sha256: &str, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit || crate::assets::sha256_hex(&bytes) != sha256 {
        bail!("cached model/license is oversized or does not match the pinned manifest hash");
    }
    Ok(bytes)
}
fn check(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        bail!("packaging command failed: {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_zip_preserves_unicode_resources_and_crc() {
        let root = std::env::temp_dir().join(format!("pdfcraft-zip-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let app = root.join("PdfCraft");
        fs::create_dir(&app).unwrap();
        fs::write(app.join("API翻译使用说明.md"), "中译英 / 英译中").unwrap();
        fs::write(app.join("pcb-glossary-public.json"), b"[]").unwrap();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip_tree(&mut writer, &root, &app).unwrap();
        let bytes = writer.finish().unwrap();
        let mut reader = zip::ZipArchive::new(bytes).unwrap();
        assert_eq!(reader.len(), 2);
        let mut text = String::new();
        reader.by_name("PdfCraft/API翻译使用说明.md").unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "中译英 / 英译中");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn win64_validation_rejects_wrong_architecture_and_truncated_or_hostile_headers() {
        let mut pe = vec![0_u8; 160];
        pe[..2].copy_from_slice(b"MZ");
        pe[60..64].copy_from_slice(&64_u32.to_le_bytes());
        pe[64..70].copy_from_slice(b"PE\0\0\x64\x86");
        pe[88..90].copy_from_slice(b"\x0b\x02");
        pe[156..158].copy_from_slice(b"\x02\x00");
        verify_win64(std::io::Cursor::new(&pe)).unwrap();
        pe[68] = 0x4c;
        assert!(verify_win64(std::io::Cursor::new(&pe)).is_err());
        pe[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(verify_win64(std::io::Cursor::new(&pe)).is_err());
        assert!(verify_win64(std::io::Cursor::new(b"MZ")).is_err());
    }
    #[test]
    fn cached_resources_are_bounded_and_hash_verified_before_packaging() {
        let hash = crate::assets::sha256_hex(b"test-model");
        assert_eq!(verified_bytes(&b"test-model"[..], &hash, 10).unwrap(), b"test-model");
        assert!(verified_bytes(&b"changed"[..], &hash, 10).is_err());
        assert!(verified_bytes(&b"test-model"[..], &hash, 9).is_err());
    }
}

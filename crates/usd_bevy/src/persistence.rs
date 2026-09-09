//! Same-directory publication of exported USD layers.

use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};

pub(crate) fn export_layer(layer: &openusd::sdf::Layer, filename: &str) -> Result<()> {
    write_atomic(filename, |temporary| Ok(layer.export(temporary)?))
}

fn write_atomic(filename: &str, write: impl FnOnce(&str) -> Result<()>) -> Result<()> {
    let target = Path::new(filename);
    let extension = target.extension().and_then(|value| value.to_str())
        .filter(|value| !value.is_empty()).context("save destination requires a file extension")?;
    let permissions = match fs::symlink_metadata(target) {
        Ok(metadata) => {
            ensure!(metadata.file_type().is_file(), "save destination must be a regular file, not a symlink or directory");
            ensure!(!metadata.permissions().readonly(), "save destination is read-only");
            Some(metadata.permissions())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("inspect save destination"),
    };
    let parent = target.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or(Path::new("."));
    #[cfg(unix)]
    let directory = fs::File::open(parent).context("open save directory")?;
    let temporary = tempfile::Builder::new().prefix(".usd-save-").suffix(&format!(".{extension}"))
        .tempfile_in(parent).context("create staged save")?;
    write(temporary.path().to_str().context("save path is not UTF-8")?).context("export staged save")?;
    if let Some(permissions) = permissions { temporary.as_file().set_permissions(permissions)?; }
    temporary.as_file().sync_all().context("sync staged save")?;
    temporary.persist(target).map_err(|error| error.error).context("publish staged save")?;
    #[cfg(unix)]
    directory.sync_all().context("save published, but syncing its directory failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_formats_reopen_and_unknown_format_preserves_existing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let stage = crate::UsdSource::new("source.usda", &b"#usda 1.0\ndef Scope \"Saved\" { double score = 7 }\n"[..])
            .unwrap().open_stage().unwrap();
        for extension in ["usda", "usdc", "usd", "usdz"] {
            let destination = directory.path().join(format!("scene.{extension}"));
            fs::write(&destination, "old content").unwrap();
            crate::authoring::save_stage_as(&stage, destination.to_str().unwrap()).unwrap();
            let bytes = fs::read(&destination).unwrap();
            match extension {
                "usda" => assert!(bytes.starts_with(b"#usda")),
                "usdz" => assert!(bytes.starts_with(b"PK")),
                _ => assert!(bytes.starts_with(b"PXR-USDC")),
            }
            let reopened = crate::UsdSource::new(&destination, bytes).unwrap().open_stage().unwrap();
            assert_eq!(reopened.prim("/Saved").unwrap().attribute("score").get::<f64>().unwrap(), Some(7.0));
        }
        let unsupported = directory.path().join("scene.unsupported");
        fs::write(&unsupported, "retain me").unwrap();
        assert!(crate::authoring::save_stage_as(&stage, unsupported.to_str().unwrap()).is_err());
        assert_eq!(fs::read_to_string(unsupported).unwrap(), "retain me");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 5);
    }

    #[cfg(unix)]
    #[test]
    fn permissions_survive_and_symlinks_and_readonly_targets_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("scene.usda");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(target.to_str().unwrap(), |temporary| Ok(fs::write(temporary, "new")?)).unwrap();
        assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o640);
        let link = directory.path().join("link.usda");
        symlink(&target, &link).unwrap();
        assert!(write_atomic(link.to_str().unwrap(), |_| panic!("must not write through symlink")).is_err());
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        fs::set_permissions(&target, fs::Permissions::from_mode(0o440)).unwrap();
        assert!(write_atomic(target.to_str().unwrap(), |_| panic!("must not overwrite read-only file")).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "new");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn failed_export_preserves_destination_and_removes_staging_file() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("scene.usda");
        fs::write(&destination, "original").unwrap();
        let error = write_atomic(destination.to_str().unwrap(), |temporary| {
            fs::write(temporary, "partial export")?;
            anyhow::bail!("simulated writer failure")
        }).unwrap_err();
        assert!(format!("{error:#}").contains("simulated writer failure"));
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        write_atomic(destination.to_str().unwrap(), |temporary| Ok(fs::write(temporary, "replacement")?)).unwrap();
        assert_eq!(fs::read_to_string(destination).unwrap(), "replacement");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_publication_cleans_up_and_preserves_existing_directory() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("scene.usda");
        let error = write_atomic(destination.to_str().unwrap(), |temporary| {
            fs::write(temporary, "complete export")?;
            fs::create_dir(&destination)?;
            Ok(())
        }).unwrap_err();
        assert!(format!("{error:#}").contains("publish staged save"));
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

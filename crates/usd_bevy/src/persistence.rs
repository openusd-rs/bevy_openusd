//! Same-directory publication of exported USD layers.

use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};

pub(crate) mod flatten;

pub(crate) fn export_layer(stage: &openusd::usd::Stage, layer: &openusd::sdf::Layer, filename: &str) -> Result<()> {
    write_atomic(filename, |temporary| {
        if Path::new(filename).extension().and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("usdz")) {
            let mut output = fs::File::create(temporary)?;
            stage.write_usdz_package(layer, &mut output)?;
        } else {
            let source_location = layer.resolved_path();
            let source_directory = source_location.and_then(|path| Path::new(path).parent()).map(directory_identity);
            let destination = std::path::absolute(filename)?;
            if source_location.is_some_and(openusd::ar::is_package_relative_path)
                || source_directory.is_some_and(|source| Some(source) != destination.parent().map(directory_identity)) {
                stage.anchored_layer(layer)?.export(temporary)?;
            } else {
                layer.export(temporary)?;
            }
        }
        Ok(())
    })
}

fn directory_identity(path: &Path) -> std::path::PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn destination_hash(path: &Path) -> Result<Option<blake3::Hash>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(metadata.file_type().is_file(), "save destination changed to a non-regular file");
            ensure!(!metadata.permissions().readonly(), "save destination is read-only");
            let mut hash = blake3::Hasher::new();
            hash.update_reader(fs::File::open(path).context("read save destination")?)?;
            Ok(Some(hash.finalize()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("inspect save destination content"),
    }
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
    let baseline = destination_hash(target)?;
    ensure!(permissions.is_some() == baseline.is_some(), "save conflict: destination changed during inspection");
    let parent = target.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or(Path::new("."));
    #[cfg(unix)]
    let directory = fs::File::open(parent).context("open save directory")?;
    let temporary = tempfile::Builder::new().prefix(".usd-save-").suffix(&format!(".{extension}"))
        .tempfile_in(parent).context("create staged save")?;
    write(temporary.path().to_str().context("save path is not UTF-8")?).context("export staged save")?;
    if let Some(permissions) = permissions { temporary.as_file().set_permissions(permissions)?; }
    temporary.as_file().sync_all().context("sync staged save")?;
    ensure!(destination_hash(target)? == baseline,
        "save conflict: destination changed during export; external content was not replaced");
    if baseline.is_some() {
        temporary.persist(target).map_err(|error| error.error).context("publish staged save")?;
    } else {
        temporary.persist_noclobber(target).map_err(|error| error.error).context("publish new staged save without replacing another file")?;
    }
    #[cfg(unix)]
    directory.sync_all().context("save published, but syncing its directory failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn atomic_save_rejects_symlink_replacement_during_export() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scene.usda");
        let foreign = directory.path().join("foreign.usda");
        std::fs::write(&path, "original").unwrap();
        std::fs::write(&foreign, "foreign").unwrap();
        let error = super::write_atomic(path.to_str().unwrap(), |staged| {
            std::fs::write(staged, "editor output")?;
            std::fs::remove_file(&path)?;
            std::os::unix::fs::symlink(&foreign, &path)?;
            Ok(())
        }).unwrap_err();
        assert!(error.to_string().contains("non-regular"), "{error:#}");
        assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&foreign).unwrap(), "foreign");
    }

    #[test]
    fn atomic_save_rejects_destination_changes_during_export() {
        for (original, external) in [(Some("original"), Some("external")),
            (Some("original"), None), (None, Some("external"))] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("scene.usda");
            if let Some(original) = original { std::fs::write(&path, original).unwrap(); }
            let error = super::write_atomic(path.to_str().unwrap(), |staged| {
                std::fs::write(staged, "editor output")?;
                if let Some(external) = external { std::fs::write(&path, external)?; }
                else { std::fs::remove_file(&path)?; }
                Ok(())
            }).unwrap_err();
            assert!(error.to_string().contains("save conflict"), "{error:#}");
            assert_eq!(std::fs::read_to_string(&path).ok().as_deref(), external);
            assert!(!std::fs::read_dir(directory.path()).unwrap().any(|entry|
                entry.unwrap().file_name().to_string_lossy().starts_with(".usd-save-")));
        }
    }

    fn check_muted_layer_exports(native: bool) {
        use crate::editor::{EditorSession, SaveMode};
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("layer_muting.usda");
        let weak = directory.path().join("layer_muting_weak.usda");
        let root_bytes = include_bytes!("../../../assets/layer_muting.usda");
        let weak_bytes = include_bytes!("../../../assets/layer_muting_weak.usda");
        std::fs::write(&root, root_bytes).unwrap();
        std::fs::write(&weak, weak_bytes).unwrap();
        let stage = crate::UsdSource::new(&root, root_bytes.as_slice()).unwrap().open_stage().unwrap();
        let mut editor = EditorSession::new(stage);
        let snapshot = editor.snapshot().unwrap();
        let weak_id = snapshot.layers.iter().find(|id| *id != &snapshot.root_layer).unwrap();
        editor.set_layer_muted(weak_id, true).unwrap();
        let before = editor.stage().root_layer().export_to_string().unwrap();
        let output = tempfile::tempdir().unwrap();
        for (label, mode, visible) in [("root", SaveMode::RootLayer, true),
            ("edit", SaveMode::EditLayer, true), ("flat", SaveMode::Flattened, false)] {
            for extension in ["usda", "usdc", "usd", "usdz"] {
                let path = output.path().join(format!("{label}.{extension}"));
                editor.save(path.to_str().unwrap(), mode).unwrap();
                let saved = if native {
                    let result = std::process::Command::new(std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into()))
                        .arg("--flatten").arg(&path).output().unwrap();
                    assert!(result.status.success(), "{label}.{extension}: {}", String::from_utf8_lossy(&result.stderr));
                    assert!(result.stderr.is_empty(), "{label}.{extension}: {}", String::from_utf8_lossy(&result.stderr));
                    crate::UsdSource::snapshot("native.usda", result.stdout).unwrap().open_stage().unwrap()
                } else {
                    crate::UsdSource::new(&path, std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap()
                };
                assert_eq!(saved.prim("/Root/Shape").unwrap().is_valid().unwrap(), visible, "{label}.{extension}");
                assert!(saved.muted_layers().is_empty());
                if !native { assert_eq!(saved.layer_stack().len(), if visible { 2 } else { 1 }); }
                if extension == "usdz" {
                    let portable = crate::UsdSource::snapshot("relocated/package.usdz", std::fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
                    assert_eq!(portable.prim("/Root/Shape").unwrap().is_valid().unwrap(), visible);
                    assert_eq!(portable.layer_stack().len(), if visible { 2 } else { 1 });
                }
                assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
                assert!(editor.stage().is_layer_muted(weak_id));
            }
        }
        assert_eq!(std::fs::read(root).unwrap(), root_bytes);
        assert_eq!(std::fs::read(weak).unwrap(), weak_bytes);
    }

    #[test]
    fn muted_layer_exports_preserve_authored_vs_composed_semantics() {
        check_muted_layer_exports(false);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_muted_layer_composition() {
        check_muted_layer_exports(true);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat"]
    fn native_export_baked_clip_values() {
        super::flatten::tests::verify_baked_clips(true);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat"]
    fn native_export_baked_clip_assets() {
        super::flatten::tests::verify_baked_clip_assets(true);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat"]
    fn native_export_nested_retimed_instances() {
        super::flatten::tests::verify_nested_retimed_instances(true);
    }

    use super::*;

    fn relocated_fixture(directory: &Path) -> crate::editor::EditorSession {
        fs::create_dir(directory.join("layers")).unwrap();
        fs::write(directory.join("layers/texture.bin"), b"external texture").unwrap();
        fs::write(directory.join("layers/asset.usda"), "#usda 1.0\ndef Scope \"Asset\" {\n double score = 29\n}\n").unwrap();
        fs::write(directory.join("layers/weak.usda"), r#"#usda 1.0
def Scope "Model" (prepend references = @./asset.usda@</Asset>) {
    asset texture = @./texture.bin@
    asset animated.timeSamples = { 0: @./texture.bin@, 1: @./future.bin@ }
}
"#).unwrap();
        let root = "#usda 1.0\n( subLayers = [@./layers/weak.usda@] )\n";
        let path = directory.join("root.usda");
        fs::write(&path, root).unwrap();
        crate::editor::EditorSession::new(crate::UsdSource::new(path, root.as_bytes()).unwrap().open_stage().unwrap())
    }

    #[test]
    fn ordinary_save_as_preserves_external_dependencies_and_live_layers() {
        use crate::editor::SaveMode;
        use openusd::sdf::Value;
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let editor = relocated_fixture(source.path());
        let weak = source.path().join("layers/weak.usda");
        let root_before = editor.stage().root_layer().export_to_string().unwrap();
        let weak_before = editor.stage().layer(weak.to_str().unwrap()).unwrap().export_to_string().unwrap();
        for mode in [SaveMode::RootLayer, SaveMode::EditLayer] {
            editor.set_edit_layer(weak.to_str().unwrap()).unwrap();
            for extension in ["usda", "usdc", "usd"] {
                let path = output.path().join(format!("moved.{extension}"));
                editor.save(path.to_str().unwrap(), mode).unwrap();
                let saved = crate::UsdSource::new(&path, fs::read(&path).unwrap()).unwrap().open_stage().unwrap();
                let prim = saved.prim("/Model").unwrap();
                assert_eq!(prim.attribute("score").get::<f64>().unwrap(), Some(29.0));
                let Value::AssetPath(texture) = prim.attribute("texture").get::<Value>().unwrap().unwrap() else { panic!() };
                assert_eq!(fs::read(texture.resolved_path().unwrap()).unwrap(), b"external texture");
                let Value::AssetPath(future) = prim.attribute("animated").get_at::<Value>(openusd::usd::TimeCode::new(1.0)).unwrap().unwrap() else { panic!() };
                if matches!(mode, SaveMode::EditLayer) { assert!(future.authored_path.ends_with("/layers/future.bin")); }
                else { assert_eq!(future.authored_path, "./future.bin"); }
            }
        }
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), root_before);
        assert_eq!(editor.stage().layer(weak.to_str().unwrap()).unwrap().export_to_string().unwrap(), weak_before);
        editor.stage().prim("/Model").unwrap().attribute("texture")
            .set(Value::AssetPath(openusd::sdf::AssetPath::new("./tile.<UDIM>.png"))).unwrap();
        let target = output.path().join("keep.usda");
        fs::write(&target, b"existing destination").unwrap();
        let error = editor.save(target.to_str().unwrap(), SaveMode::EditLayer).unwrap_err();
        assert!(format!("{error:#}").contains("unsupported relocated asset"));
        assert_eq!(fs::read(target).unwrap(), b"existing destination");
        fs::create_dir(source.path().join("layers/alias")).unwrap();
        let same_directory = source.path().join("layers/alias/../same.usda");
        editor.save(same_directory.to_str().unwrap(), SaveMode::EditLayer).unwrap();
        assert!(fs::read_to_string(same_directory).unwrap().contains("@./tile.<UDIM>.png@"));
        #[cfg(unix)]
        {
            let alias = source.path().join("linked-layers");
            std::os::unix::fs::symlink(source.path().join("layers"), &alias).unwrap();
            let path = alias.join("same-linked.usda");
            editor.save(path.to_str().unwrap(), SaveMode::EditLayer).unwrap();
            assert!(fs::read_to_string(path).unwrap().contains("@./tile.<UDIM>.png@"));
        }
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_reopens_captured_reference_batch() {
        use crate::{UsdSource, editor::{EditorEdit, EditorSession, SaveMode}};
        use openusd::sdf::Value;
        use std::io::Read;
        let input = tempfile::tempdir().unwrap();
        let root = UsdSource::snapshot(input.path().join("root.usda"), &b"#usda 1.0\n"[..]).unwrap();
        let asset = UsdSource::snapshot(input.path().join("data/payload.bin"), &b"captured payload"[..]).unwrap();
        let model = UsdSource::snapshot(input.path().join("model.usda"), &br#"#usda 1.0
(defaultPrim = "Model")
def Sphere "Model" {
    double radius = 1.5
    custom asset payload = @data/payload.bin@
}
"#[..]).unwrap().with_dependency(&asset).unwrap();
        let assembly = root.with_references([
            ("/Default", &model, openusd::sdf::Path::default()),
            ("/Explicit", &model, openusd::sdf::path("/Model").unwrap()),
        ]).unwrap();
        let customized = assembly.with_edits([
            EditorEdit::Attribute { prim: "/Explicit".into(), name: "radius".into(), type_name: "double".into(), value: Value::Double(3.) },
            EditorEdit::AttributeSample { prim: "/Explicit".into(), name: "radius".into(), type_name: "double".into(), value: Value::Double(4.), time: 2. },
        ]).unwrap();
        assert_eq!(assembly.open_stage().unwrap().prim("/Explicit").unwrap().attribute("radius").get::<f64>().unwrap(), Some(1.5));
        let editor = EditorSession::new(customized.open_stage().unwrap());
        let output = tempfile::tempdir().unwrap();
        let package = output.path().join("assembly.usdz");
        editor.save(package.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        assert_eq!(fs::read_dir(input.path()).unwrap().count(), 0);
        let relocated = tempfile::tempdir().unwrap();
        let moved = relocated.path().join("moved.usdz");
        fs::rename(&package, &moved).unwrap();
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        let result = std::process::Command::new(native).arg("--flatten").arg(&moved).output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert!(result.stderr.is_empty(), "{}", String::from_utf8_lossy(&result.stderr));
        let stage = UsdSource::snapshot(relocated.path().join("native.usda"), result.stdout).unwrap().open_stage().unwrap();
        let mut entries = std::collections::BTreeSet::new();
        for path in ["/Default", "/Explicit"] {
            let prim = stage.prim(path).unwrap();
            assert_eq!(prim.type_name().unwrap().as_deref(), Some("Sphere"));
            assert_eq!(prim.attribute("radius").get::<f64>().unwrap(), Some(if path == "/Explicit" { 3. } else { 1.5 }));
            if path == "/Explicit" {
                assert_eq!(prim.attribute("radius").get_at::<f64>(Some(openusd::usd::TimeCode::new(2.))).unwrap(), Some(4.));
            }
            let openusd::sdf::Value::AssetPath(asset) = prim.attribute("payload").get::<openusd::sdf::Value>().unwrap().unwrap() else { panic!() };
            let (outer, entry) = openusd::ar::split_package_relative_path_outer(&asset.authored_path).unwrap();
            assert_eq!(Path::new(&outer), moved.as_path());
            let mut archive = zip::ZipArchive::new(fs::File::open(&outer).unwrap()).unwrap();
            let mut bytes = Vec::new();
            archive.by_name(&entry).unwrap().read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"captured payload");
            entries.insert(entry);
        }
        assert_eq!(entries.len(), 1);
        assert_eq!(root.dependencies().count(), 0);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_instanceable_reference_contracts() {
        use crate::{UsdSource, editor::{EditorSession, SaveMode}};
        use openusd::sdf::{LayerOffset, Path as SdfPath};
        let input = tempfile::tempdir().unwrap();
        let root = UsdSource::snapshot(input.path().join("root.usda"), &b"#usda 1.0\n"[..]).unwrap();
        let model = UsdSource::snapshot(input.path().join("model.usda"), &br#"#usda 1.0
(defaultPrim = "Model")
def Xform "Model" {
    def Cube "Geometry" { double size = 2 }
}
"#[..]).unwrap();
        let assembly = root.with_instanceable_references([
            ("/First", &model, SdfPath::default(), LayerOffset::IDENTITY),
            ("/Second", &model, SdfPath::default(), LayerOffset::IDENTITY),
        ]).unwrap();
        let editor = EditorSession::new(assembly.open_stage().unwrap());
        let before = editor.stage().root_layer().export_to_string().unwrap();
        let output = tempfile::tempdir().unwrap();
        let relocated = tempfile::tempdir().unwrap();
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer), ("flat", SaveMode::Flattened)] {
            let package = output.path().join(format!("{name}.usdz"));
            editor.save(package.to_str().unwrap(), mode).unwrap();
            let moved = relocated.path().join(format!("{name}.usdz"));
            fs::rename(&package, &moved).unwrap();
            assert_eq!(fs::read_dir(input.path()).unwrap().count(), 0);
            let result = std::process::Command::new(&native).arg("--flatten").arg(&moved).output().unwrap();
            assert!(result.status.success(), "{name}: {}", String::from_utf8_lossy(&result.stderr));
            assert!(result.stderr.is_empty(), "{name}: {}", String::from_utf8_lossy(&result.stderr));
            let stage = UsdSource::snapshot(relocated.path().join("native.usda"), result.stdout).unwrap().open_stage().unwrap();
            let first = stage.prim("/First").unwrap();
            let second = stage.prim("/Second").unwrap();
            assert!(first.is_instance().unwrap(), "{name}");
            assert!(second.is_instance().unwrap(), "{name}");
            assert!(first.prototype().unwrap().is_some(), "{name}");
            assert_eq!(first.prototype().unwrap(), second.prototype().unwrap(), "{name}");
            for path in ["/First/Geometry", "/Second/Geometry"] {
                let prim = stage.prim(path).unwrap();
                assert!(prim.is_instance_proxy().unwrap(), "{name}: {path}");
                assert_eq!(prim.type_name().unwrap().as_deref(), Some("Cube"));
                assert_eq!(prim.attribute("size").get::<f64>().unwrap(), Some(2.0));
            }
        }
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_preserves_retimed_reference_batches() {
        use crate::{UsdSource, editor::{EditorSession, SaveMode}};
        use openusd::sdf::{LayerOffset, Path as SdfPath};
        let input = tempfile::tempdir().unwrap();
        let root = UsdSource::snapshot(input.path().join("root.usda"), &b"#usda 1.0\n"[..]).unwrap();
        let model = UsdSource::snapshot(input.path().join("model.usda"), &br#"#usda 1.0
(defaultPrim = "Model")
def Sphere "Model" {
    double radius.timeSamples = {0: 1, 10: 3}
}
"#[..]).unwrap();
        let assembly = root.with_offset_references([
            ("/Original", &model, SdfPath::default(), LayerOffset::IDENTITY),
            ("/Retimed", &model, SdfPath::default(), LayerOffset::new(10.0, 2.0)),
        ]).unwrap();
        let editor = EditorSession::new(assembly.open_stage().unwrap());
        let before = editor.stage().root_layer().export_to_string().unwrap();
        let output = tempfile::tempdir().unwrap();
        let relocated = tempfile::tempdir().unwrap();
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer), ("flat", SaveMode::Flattened)] {
            let package = output.path().join(format!("{name}.usdz"));
            editor.save(package.to_str().unwrap(), mode).unwrap();
            let moved = relocated.path().join(format!("{name}.usdz"));
            fs::rename(&package, &moved).unwrap();
            assert_eq!(fs::read_dir(input.path()).unwrap().count(), 0);
            let result = std::process::Command::new(&native).arg("--flatten").arg(&moved).output().unwrap();
            assert!(result.status.success(), "{name}: {}", String::from_utf8_lossy(&result.stderr));
            assert!(result.stderr.is_empty(), "{name}: {}", String::from_utf8_lossy(&result.stderr));
            let stage = UsdSource::snapshot(relocated.path().join("native.usda"), result.stdout).unwrap().open_stage().unwrap();
            for (path, times, values) in [("/Original", [0.0, 5.0, 10.0], [1.0, 2.0, 3.0]),
                ("/Retimed", [10.0, 20.0, 30.0], [1.0, 2.0, 3.0])] {
                let attribute = stage.prim(path).unwrap().attribute("radius");
                let keys: Vec<_> = attribute.time_samples().unwrap().unwrap().into_iter().map(|(time, _)| time).collect();
                assert_eq!(keys, vec![times[0], times[2]], "{name}: {path}");
                for (time, value) in times.into_iter().zip(values) {
                    assert_eq!(attribute.get_at::<f64>(Some(openusd::usd::TimeCode::new(time))).unwrap(), Some(value), "{name}: {path} at {time}");
                }
            }
        }
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_preserves_editor_affine_reset_command() {
        use crate::editor::{EditorEdit, EditorSession, SaveMode};
        let stage = crate::UsdSource::new("native-matrix-edit.usda", include_bytes!("../../../assets/xform_animation.usda").as_slice())
            .unwrap().open_stage().unwrap();
        let mut editor = EditorSession::new(stage);
        let matrix = [1.0,0.0,0.5,0.0, 0.75,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 3.0,4.0,5.0,1.0];
        editor.edit(EditorEdit::TransformMatrix { prim: "/Affine".into(), matrix, reset: true }).unwrap();
        let output = tempfile::tempdir().unwrap();
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer), ("flat", SaveMode::Flattened)] {
            for extension in ["usda", "usdc", "usd"] {
                let path = output.path().join(format!("{name}.{extension}"));
                editor.save(path.to_str().unwrap(), mode).unwrap();
                let result = std::process::Command::new(&native).arg("--flatten").arg(&path).output().expect("launch native USD_CAT or usdcat");
                assert!(result.status.success(), "{}: {}", path.display(), String::from_utf8_lossy(&result.stderr));
                let native_stage = crate::UsdSource::new(output.path().join("native-flat.usda"), result.stdout)
                    .unwrap().open_stage().unwrap();
                let prim = openusd::sdf::path("/Affine").unwrap();
                for time in [0.0, 2.5, 5.0, 10.0] {
                    assert_eq!(crate::read::xform::read_transform_stack_at(&native_stage, &prim, Some(time)).unwrap(), Some((matrix.map(|v| v as f32), true)));
                }
                assert!(native_stage.attribute("/Affine.xformOp:transform").unwrap().time_samples().unwrap().unwrap_or_default().is_empty());
                assert!(native_stage.prim("/Affine/Following").unwrap().is_valid().unwrap());
                assert!(native_stage.prim("/Affine/Reset").unwrap().is_valid().unwrap());
            }
        }
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_preserves_cross_directory_layer_dependencies() {
        use crate::editor::SaveMode;
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let editor = relocated_fixture(source.path());
        editor.set_edit_layer(source.path().join("layers/weak.usda").to_str().unwrap()).unwrap();
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        for mode in [SaveMode::RootLayer, SaveMode::EditLayer] {
            for extension in ["usda", "usdc", "usd"] {
                let path = output.path().join(format!("moved.{extension}"));
                editor.save(path.to_str().unwrap(), mode).unwrap();
                let result = std::process::Command::new(&native).arg("--flatten").arg(&path).output().unwrap();
                assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
                assert!(!String::from_utf8_lossy(&result.stderr).contains("Could not open"));
                let stage = crate::UsdSource::new(output.path().join("native.usda"), result.stdout).unwrap().open_stage().unwrap();
                assert_eq!(stage.prim("/Model").unwrap().attribute("score").get::<f64>().unwrap(), Some(29.0));
                let openusd::sdf::Value::AssetPath(texture) = stage.prim("/Model").unwrap().attribute("texture")
                    .get::<openusd::sdf::Value>().unwrap().unwrap() else { panic!() };
                assert_eq!(fs::read(texture.resolved_path().unwrap()).unwrap(), b"external texture");
            }
        }
        fs::write(source.path().join("layers/future.bin"), b"future texture").unwrap();
        let package = output.path().join("input.usdz");
        editor.save(package.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        let packaged = crate::editor::EditorSession::new(crate::UsdSource::new(&package, fs::read(&package).unwrap()).unwrap().open_stage().unwrap());
        assert!(openusd::ar::is_package_relative_path(packaged.stage().root_layer().resolved_path().unwrap()));
        let root = packaged.stage().root_layer().identifier().to_owned();
        let weak = packaged.stage().layer_stack().into_iter().find(|identifier| identifier != &root).unwrap();
        packaged.set_edit_layer(&weak).unwrap();
        for (mode, extension) in [SaveMode::RootLayer, SaveMode::EditLayer].into_iter()
            .flat_map(|mode| ["usda", "usdc", "usd"].map(|extension| (mode, extension))) {
            let path = output.path().join(format!("unwrapped.{extension}"));
            packaged.save(path.to_str().unwrap(), mode).unwrap();
            let result = std::process::Command::new(&native).arg("--flatten").arg(&path).output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            assert!(!String::from_utf8_lossy(&result.stderr).contains("Could not open"));
            let stage = crate::UsdSource::new(output.path().join("native.usda"), result.stdout).unwrap().open_stage().unwrap();
            assert_eq!(stage.prim("/Model").unwrap().attribute("score").get::<f64>().unwrap(), Some(29.0));
            let openusd::sdf::Value::AssetPath(texture) = stage.prim("/Model").unwrap().attribute("texture")
                .get::<openusd::sdf::Value>().unwrap().unwrap() else { panic!() };
            let (outer, entry) = openusd::ar::split_package_relative_path_outer(&texture.authored_path).unwrap();
            assert_eq!(Path::new(&outer), package.as_path());
            let mut archive = zip::ZipArchive::new(fs::File::open(outer).unwrap()).unwrap();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut archive.by_name(&entry).unwrap(), &mut bytes).unwrap();
            assert_eq!(bytes, b"external texture");
        }
    }

    fn variant_payload_fixture(directory: &Path) -> crate::editor::EditorSession {
        let root = r#"#usda 1.0
def Scope "Model" (
    prepend variantSets = ["choice"]
    variants = { string choice = "a" }
) {
    variantSet "choice" = {
        "a" {
            def Scope "Content" (prepend payload = @a.usda@</Asset>) {}
        }
        "b" {
            def Scope "Content" (prepend payload = @b.usda@</Asset>) {}
        }
    }
}
"#;
        for (name, other, score) in [("a", "b", 13), ("b", "a", 29)] {
            fs::write(directory.join(format!("{name}.usda")), format!(
                "#usda 1.0\ndef Scope \"Asset\" {{\n double score = {score}\n asset blob = @{name}.bin@\n asset backlink = @{other}.usda@\n}}\n"
            )).unwrap();
            fs::write(directory.join(format!("{name}.bin")), format!("payload {name}")).unwrap();
        }
        let path = directory.join("root.usda");
        fs::write(&path, root).unwrap();
        crate::editor::EditorSession::new(crate::UsdSource::new(path, root.as_bytes()).unwrap().open_stage().unwrap())
    }

    #[test]
    fn package_preserves_unloaded_variant_payloads_and_asset_cycles() {
        use crate::editor::SaveMode;
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let mut editor = variant_payload_fixture(source.path());
        editor.set_payload_loaded("/Model/Content", false).unwrap();
        let before = editor.stage().root_layer().export_to_string().unwrap();
        let path = output.path().join("unloaded.usdz");
        editor.save(path.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), before);
        let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        assert_eq!(archive.len(), 5);
        let mut blobs = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            if entry.name().ends_with(".bin") {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
                blobs.push(bytes);
            }
        }
        blobs.sort();
        assert_eq!(blobs, [b"payload a".to_vec(), b"payload b".to_vec()]);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_preserves_variant_payloads_after_source_removal() {
        use crate::editor::SaveMode;
        use std::{io::Read, process::Command};
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let mut editor = variant_payload_fixture(source.path());
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        let control = Command::new(&native).arg("--flatten").arg(source.path().join("root.usda")).output().unwrap();
        assert!(control.status.success(), "{}", String::from_utf8_lossy(&control.stderr));
        for loaded in [true, false] {
            editor.set_payload_loaded("/Model/Content", loaded).unwrap();
            for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer)] {
                editor.save(output.path().join(format!("{name}-{loaded}.usdz")).to_str().unwrap(), mode).unwrap();
            }
        }
        editor.set_payload_loaded("/Model/Content", true).unwrap();
        editor.save(output.path().join("flat.usdz").to_str().unwrap(), SaveMode::Flattened).unwrap();
        drop(editor);
        source.close().unwrap();
        for name in ["root-true", "edit-true", "root-false", "edit-false", "flat"] {
            for (choice, expected) in [("a", 13.0), ("b", 29.0)] {
                let (asset_choice, expected) = if name == "flat" { ("a", 13.0) } else { (choice, expected) };
                let wrapper = output.path().join("selection.usda");
                fs::write(&wrapper, format!(
                    "#usda 1.0\n( subLayers = [@{name}.usdz@] )\nover \"Model\" (variants = {{ string choice = \"{choice}\" }}) {{}}\n"
                )).unwrap();
                let result = Command::new(&native).arg("--flatten").arg(&wrapper).output().unwrap();
                assert!(result.status.success(), "{name}/{choice}: {}", String::from_utf8_lossy(&result.stderr));
                let stage = crate::UsdSource::new(output.path().join("native.usda"), result.stdout).unwrap().open_stage().unwrap();
                let prim = stage.prim("/Model/Content").unwrap();
                assert_eq!(prim.attribute("score").get::<f64>().unwrap(), Some(expected), "{name}/{choice}");
                let asset = prim.attribute("blob").get::<openusd::sdf::Value>().unwrap().unwrap();
                let openusd::sdf::Value::AssetPath(asset) = asset else { panic!("expected asset") };
                let (_, entry) = openusd::ar::split_package_relative_path_outer(&asset.authored_path).unwrap();
                let mut archive = zip::ZipArchive::new(fs::File::open(output.path().join(format!("{name}.usdz"))).unwrap()).unwrap();
                assert_eq!(archive.len(), 5, "{name}/{choice}: cycle must not duplicate layers");
                let mut bytes = Vec::new();
                archive.by_name(&entry).unwrap().read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, format!("payload {asset_choice}").as_bytes());
                let backlink = prim.attribute("backlink").get::<openusd::sdf::Value>().unwrap().unwrap();
                let openusd::sdf::Value::AssetPath(backlink) = backlink else { panic!("expected layer asset") };
                let (_, entry) = openusd::ar::split_package_relative_path_outer(&backlink.authored_path).unwrap();
                let mut bytes = Vec::new();
                archive.by_name(&entry).unwrap().read_to_end(&mut bytes).unwrap();
                let linked = crate::UsdSource::new(output.path().join("linked.usdc"), bytes).unwrap().open_stage().unwrap();
                assert_eq!(linked.prim("/Asset").unwrap().attribute("score").get::<f64>().unwrap(),
                    Some(if asset_choice == "a" { 29.0 } else { 13.0 }), "{name}/{choice}: backlink target");
            }
        }
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_repackages_snapshot_archives() {
        use crate::editor::{EditorSession, SaveMode};
        use std::io::{Cursor, Read};
        use std::process::Command;

        let directory = tempfile::tempdir().unwrap();
        let mut archive = openusd::usdz::ArchiveWriter::new(Cursor::new(Vec::new()));
        archive.add_layer("root.usda", b"#usda 1.0\n( subLayers = [@layers/weak.usda@] )\ndef Scope \"Assets\" {\n asset blob = @textures/pixels.bin@\n}\n").unwrap();
        archive.add_layer("layers/weak.usda", b"#usda 1.0\ndef Scope \"Weak\" {\n double score = 41\n asset blob = @../textures/pixels.bin@\n}\n").unwrap();
        archive.add_layer("textures/pixels.bin", b"packaged payload").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let virtual_path = directory.path().join("never-written.USDZ");
        let source = crate::UsdSource::snapshot(&virtual_path, bytes).unwrap();
        let editor = EditorSession::new(source.open_stage().unwrap());
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        let check = |path: &Path| {
            let result = Command::new(&native).arg("--flatten").arg(path).output().expect("native USD reader");
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            let stage = crate::UsdSource::new(directory.path().join("native.usda"), result.stdout).unwrap().open_stage().unwrap();
            assert_eq!(stage.prim("/Weak").unwrap().attribute("score").get::<f64>().unwrap(), Some(41.0));
            let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
            for prim in ["/Weak", "/Assets"] {
                let value = stage.prim(prim).unwrap().attribute("blob").get::<openusd::sdf::Value>().unwrap().unwrap();
                let openusd::sdf::Value::AssetPath(asset) = value else { panic!("expected asset") };
                let (_, entry) = openusd::ar::split_package_relative_path_outer(&asset.authored_path).expect("packaged asset");
                let mut bytes = Vec::new();
                archive.by_name(&entry).unwrap().read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, b"packaged payload");
            }
        };
        for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer), ("flat", SaveMode::Flattened)] {
            let first = directory.path().join(format!("{name}.usdz"));
            editor.save(first.to_str().unwrap(), mode).unwrap();
            check(&first);
            let source = crate::UsdSource::snapshot(&first, fs::read(&first).unwrap()).unwrap();
            let second = directory.path().join(format!("{name}-repacked.usdz"));
            let second_editor = EditorSession::new(source.open_stage().unwrap());
            fs::remove_file(&first).unwrap();
            second_editor.save(second.to_str().unwrap(), SaveMode::RootLayer).unwrap();
            check(&second);
        }
        let wrapper = b"#usda 1.0\ndef Scope \"Weak\" (\n prepend references = @never-written.USDZ@</Weak>\n) {}\ndef Scope \"Assets\" (\n prepend references = @never-written.USDZ@</Assets>\n) {}\n";
        let mut wrapper_source = crate::UsdSource::snapshot(&directory.path().join("wrapper.usda"), wrapper.to_vec()).unwrap();
        wrapper_source.insert_dependency(virtual_path.to_str().unwrap().into(), source.read_asset(source.identifier()).unwrap());
        let wrapped = directory.path().join("wrapped.usdz");
        crate::authoring::save_stage_as(&wrapper_source.open_stage().unwrap(), wrapped.to_str().unwrap()).unwrap();
        assert_eq!(zip::ZipArchive::new(fs::File::open(&wrapped).unwrap()).unwrap().len(), 4);
        check(&wrapped);
        assert!(!virtual_path.exists());
    }

    #[test]
    fn package_preserves_snapshot_assets_live_edits_and_archive_layout() {
        use crate::editor::{EditorEdit, EditorSession, SaveMode};
        use openusd::sdf::Value;
        use std::io::Read;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("virtual/root.usda");
        let weak = directory.path().join("virtual/weak.usda");
        let source_text = b"#usda 1.0\n( subLayers = [@weak.usda@] )\ndef Scope \"Assets\" {\n asset first = @a/color.png@\n asset second = @b/color.png@\n asset repeated = @a/color.png@\n asset self = @root.usda@\n}\n";
        let mut source = crate::UsdSource::snapshot(&root, source_text.to_vec()).unwrap();
        source.insert_dependency(weak.to_str().unwrap().into(), b"#usda 1.0\ndef Scope \"Weak\" {\n double score = 11\n}\n".to_vec());
        source.insert_dependency(directory.path().join("virtual/a/color.png").to_str().unwrap().into(), b"first image".to_vec());
        source.insert_dependency(directory.path().join("virtual/b/color.png").to_str().unwrap().into(), b"second image".to_vec());
        let mut editor = EditorSession::new(source.open_stage().unwrap());
        editor.set_edit_layer(weak.to_str().unwrap()).unwrap();
        editor.edit(EditorEdit::Attribute { prim: "/Weak".into(), name: "score".into(), type_name: "double".into(), value: Value::Double(31.0) }).unwrap();
        let root_before = editor.stage().root_layer().export_to_string().unwrap();
        let weak_before = editor.stage().layer(weak.to_str().unwrap()).unwrap().export_to_string().unwrap();
        let undo_before = editor.snapshot().unwrap().can_undo;
        let output = directory.path().join("saved.USDZ");
        editor.save(output.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        let first = fs::read(&output).unwrap();
        editor.save(output.to_str().unwrap(), SaveMode::RootLayer).unwrap();
        assert_eq!(fs::read(&output).unwrap(), first);
        assert_eq!(editor.stage().root_layer().export_to_string().unwrap(), root_before);
        assert_eq!(editor.stage().layer(weak.to_str().unwrap()).unwrap().export_to_string().unwrap(), weak_before);
        assert_eq!(editor.snapshot().unwrap().can_undo, undo_before);
        assert!(!root.exists());
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&first)).unwrap();
        assert_eq!(archive.len(), 4);
        let mut images = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            if index == 0 { assert_eq!(entry.name(), "scene.usdc"); }
            assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
            assert_eq!(entry.data_start().unwrap() % 64, 0);
            assert!(!entry.name().contains(['/', '\\']));
            if entry.name().ends_with(".png") {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                images.push(bytes);
            }
        }
        images.sort();
        assert_eq!(images, vec![b"first image".to_vec(), b"second image".to_vec()]);
        let reopened = crate::UsdSource::new(&output, first).unwrap().open_stage().unwrap();
        assert_eq!(reopened.prim("/Weak").unwrap().attribute("score").get::<f64>().unwrap(), Some(31.0));
        assert!(editor.undo().unwrap());
        assert_eq!(editor.stage().prim("/Weak").unwrap().attribute("score").get::<f64>().unwrap(), Some(11.0));
        assert!(!editor.snapshot().unwrap().can_undo);
    }

    #[test]
    fn package_entry_budget_preserves_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let paths = (0..4096).map(|index| format!("@asset{index}.bin@")).collect::<Vec<_>>().join(", ");
        let text = format!("#usda 1.0\ndef Scope \"Assets\" {{\n asset[] files = [{paths}]\n}}\n");
        let mut source = crate::UsdSource::snapshot(&directory.path().join("virtual.usda"), text.into_bytes()).unwrap();
        for index in 0..4096 {
            source.insert_dependency(directory.path().join(format!("asset{index}.bin")).to_str().unwrap().into(), vec![1]);
        }
        let output = directory.path().join("saved.usdz");
        fs::write(&output, b"original").unwrap();
        let error = crate::authoring::save_stage_as(&source.open_stage().unwrap(), output.to_str().unwrap()).unwrap_err();
        assert!(format!("{error:#}").contains("entry budget exceeded"));
        assert_eq!(fs::read(&output).unwrap(), b"original");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_package_asset_preserves_destination_and_cleans_staging() {
        let directory = tempfile::tempdir().unwrap();
        let source = crate::UsdSource::new(directory.path().join("source.usda"),
            &b"#usda 1.0\ndef Scope \"Assets\" {\n asset texture = @missing.png@\n}\n"[..]).unwrap();
        let output = directory.path().join("saved.usdz");
        fs::write(&output, b"original").unwrap();
        let error = crate::authoring::save_stage_as(&source.open_stage().unwrap(), output.to_str().unwrap()).unwrap_err();
        assert!(format!("{error:#}").contains("missing.png"));
        assert_eq!(fs::read(&output).unwrap(), b"original");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_package_survives_removal_of_layer_dependencies() {
        use crate::editor::{EditorSession, SaveMode};
        use std::process::Command;

        let source_directory = tempfile::tempdir().unwrap();
        let output_directory = tempfile::tempdir().unwrap();
        fs::write(source_directory.path().join("weak.usda"), "#usda 1.0\ndef Scope \"Layered\" {\n double score = 11\n}\n").unwrap();
        fs::write(source_directory.path().join("model.usda"), "#usda 1.0\ndef Scope \"Model\" {\n double score = 23\n asset texture = @paint.bin@\n}\n").unwrap();
        fs::write(source_directory.path().join("paint.bin"), b"texture payload").unwrap();
        let source = b"#usda 1.0\n( subLayers = [@weak.usda@] )\ndef Scope \"Referenced\" (\n prepend references = @model.usda@</Model>\n) {}\n";
        let original = source_directory.path().join("source.usda");
        fs::write(&original, source).unwrap();
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        let control = Command::new(&native).arg("--flatten").arg(&original).output().expect("launch native reader");
        assert!(control.status.success(), "native source control: {}", String::from_utf8_lossy(&control.stderr));
        let editor = EditorSession::new(crate::UsdSource::new(&original, source.as_slice()).unwrap().open_stage().unwrap());
        for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer), ("flat", SaveMode::Flattened)] {
            editor.save(output_directory.path().join(format!("{name}.usdz")).to_str().unwrap(), mode).unwrap();
        }
        drop(editor);
        source_directory.close().unwrap();
        let mut failures = Vec::new();
        for name in ["root", "edit", "flat"] {
            let package = output_directory.path().join(format!("{name}.usdz"));
            let output = Command::new(&native).arg("--flatten").arg(&package).output().expect("launch native reader");
            if !output.status.success() {
                failures.push(format!("{name}: {}", String::from_utf8_lossy(&output.stderr)));
                continue;
            }
            let stage = crate::UsdSource::new(output_directory.path().join("native.usda"), output.stdout).unwrap().open_stage().unwrap();
            for (prim, expected) in [("/Layered", 11.0), ("/Referenced", 23.0)] {
                let value = stage.prim(prim).unwrap().attribute("score").get::<f64>().unwrap();
                if value != Some(expected) { failures.push(format!("{name} {prim}: expected {expected}, got {value:?}")); }
            }
            let value = stage.prim("/Referenced").unwrap().attribute("texture").get::<openusd::sdf::Value>().unwrap();
            if let Some(openusd::sdf::Value::AssetPath(asset)) = value {
                if let Some((_, entry)) = openusd::ar::split_package_relative_path_outer(&asset.authored_path) {
                    use std::io::Read;
                    let mut archive = zip::ZipArchive::new(fs::File::open(&package).unwrap()).unwrap();
                    let mut payload = Vec::new();
                    archive.by_name(&entry).unwrap().read_to_end(&mut payload).unwrap();
                    assert_eq!(payload, b"texture payload");
                } else { failures.push(format!("{name}: texture is not package-relative: {}", asset.authored_path)); }
            } else { failures.push(format!("{name}: texture asset is missing")); }
        }
        assert!(failures.is_empty(), "non-portable package exports:\n{}", failures.join("\n"));
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_preserves_sublayers_and_external_references() {
        use crate::editor::{EditorSession, SaveMode};
        use std::process::Command;

        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("weak.usda"), "#usda 1.0\ndef Scope \"Layered\" {\n double score = 11\n}\n").unwrap();
        fs::write(directory.path().join("model.usda"), "#usda 1.0\ndef Scope \"Model\" {\n double score = 23\n}\n").unwrap();
        let source = b"#usda 1.0\n( subLayers = [@weak.usda@] )\ndef Scope \"Referenced\" (\n prepend references = @model.usda@</Model>\n) {}\n";
        let original = directory.path().join("source.usda");
        fs::write(&original, source).unwrap();
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        let check = |path: &Path| -> Result<()> {
            let output = Command::new(&native).arg("--flatten").arg(path).output().context("launch native USD reader")?;
            ensure!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let stage = crate::UsdSource::new(directory.path().join("native.usda"), output.stdout)?.open_stage()?;
            for (prim, expected) in [("/Layered", 11.0), ("/Referenced", 23.0)] {
                let value = stage.prim(prim)?.attribute("score").get::<f64>()?;
                ensure!(value == Some(expected), "{prim}: expected {expected}, got {value:?}");
            }
            Ok(())
        };
        check(&original).expect("native source control");
        let stage = crate::UsdSource::new(&original, source.as_slice()).unwrap().open_stage().unwrap();
        let editor = EditorSession::new(stage);
        let mut failures = Vec::new();
        for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer), ("flat", SaveMode::Flattened)] {
            for extension in ["usda", "usdc", "usd", "usdz"] {
                let destination = directory.path().join(format!("{name}.{extension}"));
                editor.save(destination.to_str().unwrap(), mode).unwrap();
                if let Err(error) = check(&destination) {
                    failures.push(format!("{name}.{extension}: {error:#}"));
                }
            }
        }
        assert!(failures.is_empty(), "native composition failures:\n{}", failures.join("\n"));
    }

    #[test]
    #[ignore = "requires native OpenUSD usdcat; run make test-native"]
    fn native_export_preserves_hierarchy_values_and_api_metadata() {
        use crate::editor::{EditorSession, SaveMode};
        use std::process::Command;

        let directory = tempfile::tempdir().unwrap();
        let source = include_bytes!("../../../assets/native_save.usda");
        let native = std::env::var_os("USD_CAT").unwrap_or_else(|| "usdcat".into());
        let original = directory.path().join("source.usda");
        fs::write(&original, source).unwrap();
        let control = Command::new(&native).arg(&original).output().expect("launch native USD_CAT or usdcat");
        assert!(control.status.success(), "native source control failed: {}", String::from_utf8_lossy(&control.stderr));
        let stage = crate::UsdSource::new(&original, source.as_slice()).unwrap().open_stage().unwrap();
        let editor = EditorSession::new(stage);
        let mut failures = Vec::new();
        for (name, mode) in [("root", SaveMode::RootLayer), ("edit", SaveMode::EditLayer), ("flat", SaveMode::Flattened)] {
            for extension in ["usda", "usdc", "usd", "usdz"] {
                let destination = directory.path().join(format!("{name}.{extension}"));
                editor.save(destination.to_str().unwrap(), mode).unwrap();
                let output = Command::new(&native).arg("--flatten").arg(&destination).output().expect("launch native USD reader");
                if !output.status.success() {
                    failures.push(format!("{name}.{extension}: {}", String::from_utf8_lossy(&output.stderr)));
                    continue;
                }
                let metadata = String::from_utf8_lossy(&output.stdout).contains("MaterialBindingAPI");
                let reopened = crate::UsdSource::new(directory.path().join("native.usda"), output.stdout)
                    .unwrap().open_stage().unwrap();
                let score = reopened.prim("/Saved").ok().and_then(|prim| prim.attribute("score").get::<f64>().ok().flatten());
                let size = reopened.prim("/Saved/Child").ok().and_then(|prim| prim.attribute("size").get::<f64>().ok().flatten());
                let variant = reopened.prim("/Saved").ok().and_then(|prim| prim.attribute("variantScore").get::<f64>().ok().flatten());
                if score != Some(7.0) || size != Some(3.0) || variant != Some(21.0) || !metadata {
                    failures.push(format!("{name}.{extension}: score={score:?}, child size={size:?}, variant={variant:?}, API={metadata}"));
                }
            }
        }
        assert!(failures.is_empty(), "native interchange failures:\n{}", failures.join("\n"));
    }

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
    fn conflicting_directory_cleans_up_and_preserves_external_entry() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("scene.usda");
        let error = write_atomic(destination.to_str().unwrap(), |temporary| {
            fs::write(temporary, "complete export")?;
            fs::create_dir(&destination)?;
            Ok(())
        }).unwrap_err();
        assert!(format!("{error:#}").contains("non-regular file"));
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

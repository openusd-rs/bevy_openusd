use super::Stage;
use crate::{
    ar,
    sdf::{self, AbstractData, Value},
};

impl Stage {
    /// Copies a layer with asset identifiers anchored through this stage's resolver.
    /// Asset expressions, tile/sequence patterns and clip templates return errors.
    pub fn anchored_layer(&self, layer: &sdf::Layer) -> crate::Result<sdf::Layer> {
        let graph = self.layers();
        let registry = graph.layer_registry();
        let anchor = layer.resolved_path().map(ar::ResolvedPath::new);
        let mut unsupported = None;
        let mut map = |path: &str| {
            if path.is_empty() {
                return String::new();
            }
            if path.contains(['`', '<', '#']) {
                unsupported = Some(format!("unsupported relocated asset expression/pattern: {path}"));
                return path.to_owned();
            }
            let identifier = registry.create_identifier(path, anchor.as_ref());
            registry
                .resolve(&identifier)
                .map_or(identifier, |resolved| resolved.to_string())
        };
        let mut data = sdf::Data::from_abstract(layer.data())?;
        for path in data.spec_paths() {
            for field in data.list_fields(&path).unwrap_or_default() {
                let mut value = data.get_field(&path, &field)?.into_owned();
                if field == "clips" && has_template(&value) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "relocating clip template paths is unsupported",
                    )
                    .into());
                }
                if field == "subLayers"
                    && let Value::StringVec(paths) = &mut value
                {
                    for path in paths {
                        *path = map(path);
                    }
                }
                rewrite(&mut value, &mut map);
                data.set_field(&path, &field, value);
            }
        }
        if let Some(error) = unsupported {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error).into());
        }
        Ok(sdf::Layer::new(
            sdf::Layer::anonymous_identifier("anchored.usda"),
            Box::new(data),
        ))
    }
}

fn has_template(value: &Value) -> bool {
    matches!(value, Value::Dictionary(values) if values.contains_key("templateAssetPath") || values.values().any(has_template))
}

fn rewrite(value: &mut Value, map: &mut impl FnMut(&str) -> String) {
    match value {
        Value::AssetPath(asset) => *asset = sdf::AssetPath::new(map(&asset.authored_path)),
        Value::AssetPathVec(assets) => {
            for asset in assets {
                *asset = sdf::AssetPath::new(map(&asset.authored_path));
            }
        }
        Value::Dictionary(values) => {
            for value in values.values_mut() {
                rewrite(value, map);
            }
        }
        Value::ValueVec(values) => {
            for value in values {
                rewrite(value, map);
            }
        }
        Value::TimeSamples(values) => {
            for (_, value) in values {
                rewrite(value, map);
            }
        }
        Value::ReferenceListOp(op) => list(op, |reference| {
            reference.asset_path = map(&reference.asset_path);
            for value in reference.custom_data.values_mut() {
                rewrite(value, map);
            }
        }),
        Value::PayloadListOp(op) => list(op, |payload| {
            payload.asset_path = map(&payload.asset_path);
        }),
        Value::Payload(payload) => payload.asset_path = map(&payload.asset_path),
        _ => (),
    }
}

fn list<T: Default + Clone + PartialEq>(op: &mut sdf::ListOp<T>, mut map: impl FnMut(&mut T)) {
    for items in [
        &mut op.explicit_items,
        &mut op.prepended_items,
        &mut op.appended_items,
        &mut op.deleted_items,
        &mut op.added_items,
        &mut op.ordered_items,
    ] {
        for item in items {
            map(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_list_bucket_and_nested_asset_container_is_rewritten() {
        let mut op = sdf::ListOp::<String>::default();
        for items in [
            &mut op.explicit_items,
            &mut op.prepended_items,
            &mut op.appended_items,
            &mut op.deleted_items,
            &mut op.added_items,
            &mut op.ordered_items,
        ] {
            items.push("asset".into());
        }
        let mut count = 0;
        list(&mut op, |item| {
            *item = format!("/{item}");
            count += 1;
        });
        assert_eq!(count, 6);
        assert_eq!(op.deleted_items, ["/asset"]);
        let mut value = Value::Dictionary(
            [(
                "nested".into(),
                Value::ValueVec(vec![
                    Value::AssetPath(sdf::AssetPath::new("a")),
                    Value::AssetPathVec(vec![sdf::AssetPath::new("b")]),
                    Value::TimeSamples(vec![(1.0, Value::AssetPath(sdf::AssetPath::new("c")))]),
                ]),
            )]
            .into(),
        );
        let mut paths = Vec::new();
        rewrite(&mut value, &mut |path| {
            paths.push(path.to_owned());
            format!("/{path}")
        });
        assert_eq!(paths, ["a", "b", "c"]);
        let mut rewritten = Vec::new();
        value.map_asset_paths(&mut |asset| {
            rewritten.push(asset.authored_path.clone());
            asset
        });
        assert_eq!(rewritten, ["/a", "/b", "/c"]);
    }
}

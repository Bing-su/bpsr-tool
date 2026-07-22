use std::{collections::BTreeSet, fs};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use dotnetdll::prelude::*;

#[derive(Clone, Debug)]
pub(crate) struct Table {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub name: String,
    pub kind: FieldKind,
}

#[derive(Clone, Debug)]
pub(crate) enum FieldKind {
    I32,
    I64,
    Bool,
    F32,
    String,
    Vector2,
    Vector3,
    Int32Array,
    Int64Array,
    NumberArray,
    StringArray,
    Vector2Array,
    Vector3Array,
    MlStringArray,
    Int32Table,
    StringTable,
    NumberTable,
    MlStringTable,
    StringTripleArray,
    KvIntInt,
    Other,
}

impl FieldKind {
    pub fn inline_size(&self) -> usize {
        match self {
            Self::I64 | Self::Vector2 => 8,
            Self::Vector3 => 12,
            Self::Bool => 1,
            _ => 4,
        }
    }
}

pub(crate) fn read_tables(path: &Utf8Path) -> Result<Vec<Table>> {
    let bytes = fs::read(path)?;
    let options = ReadOptions {
        skip_method_bodies: true,
        ..Default::default()
    };
    let resolution =
        Resolution::parse(&bytes, options).context("invalid .NET metadata in Panda.Table.dll")?;

    let loader = resolution
        .type_definitions
        .iter()
        .find(|ty| {
            ty.name == "<>c"
                && ty
                    .encloser
                    .is_some_and(|parent| resolution[parent].name == "TableInitUtility")
        })
        .context("Panda.TableInitUtility/<>c not found")?;

    let mut returned = BTreeSet::new();
    for method in &loader.methods {
        if method.accessibility == MemberAccessibility::Access(Accessibility::Assembly) {
            let name = method.signature.return_type.show(&resolution);
            if name != "void" {
                returned.insert(name);
            }
        }
    }

    let mut tables = Vec::new();
    for ty in &resolution.type_definitions {
        let full_name = ty.nested_type_name(&resolution);
        if !returned.contains(&full_name) && !returned.contains(ty.name.as_ref()) {
            continue;
        }
        let name = table_name(&ty.name).to_owned();
        let fields = ty
            .properties
            .iter()
            .filter(|property| {
                property.getter.as_ref().is_some_and(|getter| {
                    getter.accessibility == MemberAccessibility::Access(Accessibility::Public)
                })
            })
            .map(|property| Field {
                name: property.name.to_string(),
                kind: classify(&property.property_type.show(&resolution)),
            })
            .collect();
        tables.push(Table { name, fields });
    }
    if tables.is_empty() {
        bail!("no table return types found in Panda.Table.dll");
    }
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    tables.dedup_by(|a, b| a.name == b.name);
    Ok(tables)
}

fn table_name(name: &str) -> &str {
    // Generated table row types use either the Base or Data suffix.
    name.strip_suffix("Base")
        .or_else(|| name.strip_suffix("Data"))
        .unwrap_or(name)
}

fn classify(name: &str) -> FieldKind {
    let name = name
        .trim_start_matches("ref ")
        .trim_start_matches("valuetype ");
    // dotnetdll qualifies external value types as [Assembly]Namespace.Type.
    let name = name
        .strip_prefix('[')
        .and_then(|name| name.split_once(']'))
        .map_or(name, |(_, name)| name);
    match name {
        "System.Int32" | "int" => FieldKind::I32,
        "System.Int64" | "long" => FieldKind::I64,
        "System.Boolean" | "bool" => FieldKind::Bool,
        "System.Single" | "float" => FieldKind::F32,
        "System.String" | "string" => FieldKind::String,
        "UnityEngine.Vector2" => FieldKind::Vector2,
        "UnityEngine.Vector3" => FieldKind::Vector3,
        "Bokura.Table.Int32Array" => FieldKind::Int32Array,
        "Bokura.Table.Int64Array" => FieldKind::Int64Array,
        "Bokura.Table.NumberArray" => FieldKind::NumberArray,
        "Bokura.Table.StringArray" => FieldKind::StringArray,
        "Bokura.Table.Vector2Array" => FieldKind::Vector2Array,
        "Bokura.Table.Vector3Array" => FieldKind::Vector3Array,
        "Bokura.Table.MLStringArray" => FieldKind::MlStringArray,
        "Bokura.Table.Int32Table" => FieldKind::Int32Table,
        "Bokura.Table.StringTable" => FieldKind::StringTable,
        "Bokura.Table.NumberTable" => FieldKind::NumberTable,
        "Bokura.Table.MLStringTable" => FieldKind::MlStringTable,
        "Bokura.Table.StringTripleArray" => FieldKind::StringTripleArray,
        "Bokura.Table.KVIntInt" => FieldKind::KvIntInt,
        _ => FieldKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_known_types() {
        assert!(matches!(classify("System.Int32"), FieldKind::I32));
        assert!(matches!(
            classify("valuetype [UnityEngine.CoreModule]UnityEngine.Vector3"),
            FieldKind::Vector3
        ));
        assert_eq!(FieldKind::Vector3.inline_size(), 12);
        assert_eq!(table_name("AvatarShowTableBase"), "AvatarShowTable");
        assert_eq!(table_name("LegacyTableData"), "LegacyTable");
    }
}

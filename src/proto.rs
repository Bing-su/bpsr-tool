use std::{collections::BTreeSet, fmt::Write as _, fs};

use anyhow::{Context, Result, bail};
use camino::{Utf8Component, Utf8Path};
use prost::Message;
use prost_types::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    FileDescriptorSet, ServiceDescriptorProto,
    field_descriptor_proto::{Label, Type},
};

pub(crate) fn dump(bytes: &[u8], root: &Utf8Path) -> Result<()> {
    let set = FileDescriptorSet::decode(bytes)?;
    let names: BTreeSet<_> = set
        .file
        .iter()
        .filter_map(|file| file.name.as_deref())
        .collect();
    for file in &set.file {
        for dependency in &file.dependency {
            if !names.contains(dependency.as_str()) && !dependency.starts_with("google/protobuf/") {
                bail!("missing protobuf dependency {dependency}");
            }
        }
    }
    for file in &set.file {
        let name = file
            .name
            .as_deref()
            .context("descriptor has no file name")?;
        let relative = safe_proto_path(name)?;
        let package = file.package().split('.').filter(|part| !part.is_empty());
        let package: Vec<_> = if file.package().starts_with("google.protobuf") {
            package.skip(2).collect()
        } else {
            package.collect()
        };
        if package.iter().any(|part| !is_safe_component(part)) {
            bail!("unsafe protobuf package {}", file.package());
        }
        let path = package
            .into_iter()
            .fold(root.to_owned(), |path, part| path.join(part))
            .join(relative);
        fs::create_dir_all(path.parent().context("proto output has no parent")?)?;
        fs::write(path, format_file(file))?;
    }
    Ok(())
}

fn safe_proto_path(name: &str) -> Result<&Utf8Path> {
    let path = Utf8Path::new(name);
    if path
        .components()
        .all(|part| matches!(part, Utf8Component::Normal(_)))
    {
        Ok(path)
    } else {
        bail!("unsafe proto file name {name}")
    }
}

fn is_safe_component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\'])
}

fn format_file(file: &FileDescriptorProto) -> String {
    let mut out = String::new();
    writeln!(out, "syntax = {:?};\n", file.syntax()).unwrap();
    if !file.package().is_empty() {
        writeln!(out, "package {};\n", file.package()).unwrap();
    }
    for (index, dependency) in file.dependency.iter().enumerate() {
        let modifier = if file.public_dependency.contains(&(index as i32)) {
            "public "
        } else if file.weak_dependency.contains(&(index as i32)) {
            "weak "
        } else {
            ""
        };
        writeln!(out, "import {modifier}{dependency:?};").unwrap();
    }
    if !file.dependency.is_empty() {
        out.push('\n');
    }
    for item in &file.enum_type {
        format_enum(&mut out, item, 0);
    }
    format_extensions(&mut out, file, &file.extension, 0);
    for item in &file.message_type {
        format_message(&mut out, file, item, 0);
    }
    for item in &file.service {
        format_service(&mut out, item);
    }
    out
}

fn format_enum(out: &mut String, item: &EnumDescriptorProto, indent: usize) {
    line(out, indent, &format!("enum {} {{", item.name()));
    for value in &item.value {
        line(
            out,
            indent + 1,
            &format!("{} = {};", value.name(), value.number()),
        );
    }
    line(out, indent, "}\n");
}

fn format_message(
    out: &mut String,
    file: &FileDescriptorProto,
    item: &DescriptorProto,
    indent: usize,
) {
    line(out, indent, &format!("message {} {{", item.name()));
    for nested in &item.enum_type {
        format_enum(out, nested, indent + 1);
    }
    for nested in item.nested_type.iter().filter(|nested| {
        !nested
            .options
            .as_ref()
            .is_some_and(|options| options.map_entry())
    }) {
        format_message(out, file, nested, indent + 1);
    }
    format_extensions(out, file, &item.extension, indent + 1);
    for (index, oneof) in item.oneof_decl.iter().enumerate() {
        line(out, indent + 1, &format!("oneof {} {{", oneof.name()));
        for field in item
            .field
            .iter()
            .filter(|field| field.oneof_index == Some(index as i32))
        {
            line(out, indent + 2, &format_field(file, field, false));
        }
        line(out, indent + 1, "}");
    }
    for field in item
        .field
        .iter()
        .filter(|field| field.oneof_index.is_none())
    {
        line(out, indent + 1, &format_field(file, field, true));
    }
    for range in &item.extension_range {
        let end = if range.end() >= 536_870_911 {
            "max".to_owned()
        } else {
            (range.end() - 1).to_string()
        };
        line(
            out,
            indent + 1,
            &format!("extensions {} to {end};", range.start()),
        );
    }
    line(out, indent, "}\n");
}

fn format_field(
    file: &FileDescriptorProto,
    field: &FieldDescriptorProto,
    include_label: bool,
) -> String {
    let label = if include_label {
        match field.label() {
            Label::Optional if file.syntax() == "proto3" => "",
            Label::Optional => "optional ",
            Label::Required => "required ",
            Label::Repeated => "repeated ",
        }
    } else {
        ""
    };
    let mut ty = field
        .type_name
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_start_matches('.').to_owned())
        .unwrap_or_else(|| scalar(field.r#type()).to_owned());
    if field.label() == Label::Repeated
        && field.r#type() == Type::Message
        && let Some(entry) = find_message(file, field.type_name()).filter(|entry| {
            entry
                .options
                .as_ref()
                .is_some_and(|options| options.map_entry())
        })
        && let (Some(key), Some(value)) = (
            entry.field.iter().find(|field| field.name() == "key"),
            entry.field.iter().find(|field| field.name() == "value"),
        )
    {
        let value_type = value
            .type_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .map(|name| name.trim_start_matches('.').to_owned())
            .unwrap_or_else(|| scalar(value.r#type()).to_owned());
        ty = format!("map<{}, {value_type}>", scalar(key.r#type()));
    }
    let mut options = Vec::new();
    if let Some(name) = &field.json_name {
        options.push(format!("json_name = {name:?}"));
    }
    if let Some(field_options) = &field.options {
        if field_options.deprecated.is_some() {
            options.push(format!("deprecated = {}", field_options.deprecated()));
        }
        if field_options.packed.is_some() {
            options.push(format!("packed = {}", field_options.packed()));
        }
        if field_options.lazy.is_some() {
            options.push(format!("lazy = {}", field_options.lazy()));
        }
        if field_options.weak.is_some() {
            options.push(format!("weak = {}", field_options.weak()));
        }
    }
    let options = if options.is_empty() {
        String::new()
    } else {
        format!(" [{}]", options.join(", "))
    };
    let label = if ty.starts_with("map<") { "" } else { label };
    format!(
        "{label}{ty} {} = {}{options};",
        field.name(),
        field.number()
    )
}

fn find_message<'a>(file: &'a FileDescriptorProto, type_name: &str) -> Option<&'a DescriptorProto> {
    let name = type_name.rsplit('.').next()?;
    fn find<'a>(items: &'a [DescriptorProto], name: &str) -> Option<&'a DescriptorProto> {
        items
            .iter()
            .find(|item| item.name() == name)
            .or_else(|| items.iter().find_map(|item| find(&item.nested_type, name)))
    }
    find(&file.message_type, name)
}

fn format_extensions(
    out: &mut String,
    file: &FileDescriptorProto,
    fields: &[FieldDescriptorProto],
    indent: usize,
) {
    let extendees: BTreeSet<_> = fields.iter().map(FieldDescriptorProto::extendee).collect();
    for extendee in extendees {
        line(
            out,
            indent,
            &format!("extend {} {{", extendee.trim_start_matches('.')),
        );
        for field in fields.iter().filter(|field| field.extendee() == extendee) {
            line(out, indent + 1, &format_field(file, field, true));
        }
        line(out, indent, "}\n");
    }
}

fn scalar(ty: Type) -> &'static str {
    match ty {
        Type::Double => "double",
        Type::Float => "float",
        Type::Int64 => "int64",
        Type::Uint64 => "uint64",
        Type::Int32 => "int32",
        Type::Fixed64 => "fixed64",
        Type::Fixed32 => "fixed32",
        Type::Bool => "bool",
        Type::String => "string",
        Type::Group => "group",
        Type::Message => "message",
        Type::Bytes => "bytes",
        Type::Uint32 => "uint32",
        Type::Enum => "enum",
        Type::Sfixed32 => "sfixed32",
        Type::Sfixed64 => "sfixed64",
        Type::Sint32 => "sint32",
        Type::Sint64 => "sint64",
    }
}

fn format_service(out: &mut String, service: &ServiceDescriptorProto) {
    writeln!(out, "service {} {{", service.name()).unwrap();
    for method in &service.method {
        let client = if method.client_streaming() {
            "stream "
        } else {
            ""
        };
        let server = if method.server_streaming() {
            "stream "
        } else {
            ""
        };
        writeln!(
            out,
            "  rpc {} ({client}{}) returns ({server}{});",
            method.name(),
            method.input_type(),
            method.output_type()
        )
        .unwrap();
    }
    out.push_str("}\n\n");
}

fn line(out: &mut String, indent: usize, text: &str) {
    writeln!(out, "{}{text}", "  ".repeat(indent)).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_descriptor() {
        let file = FileDescriptorProto {
            name: Some("sample.proto".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Ping".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("id".into()),
                    number: Some(1),
                    r#type: Some(Type::Int32 as i32),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let text = format_file(&file);
        assert!(text.contains("message Ping"));
        assert!(text.contains("int32 id = 1;"));
        assert!(!text.contains("optional int32"));
    }

    #[test]
    fn formats_map_like_original() {
        let map_entry = DescriptorProto {
            name: Some("LabelsEntry".into()),
            field: vec![
                FieldDescriptorProto {
                    name: Some("key".into()),
                    number: Some(1),
                    r#type: Some(Type::String as i32),
                    ..Default::default()
                },
                FieldDescriptorProto {
                    name: Some("value".into()),
                    number: Some(2),
                    r#type: Some(Type::Int32 as i32),
                    ..Default::default()
                },
            ],
            options: Some(prost_types::MessageOptions {
                map_entry: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        let file = FileDescriptorProto {
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Data".into()),
                nested_type: vec![map_entry],
                field: vec![FieldDescriptorProto {
                    name: Some("labels".into()),
                    number: Some(1),
                    label: Some(Label::Repeated as i32),
                    r#type: Some(Type::Message as i32),
                    type_name: Some(".Data.LabelsEntry".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let text = format_file(&file);
        assert!(text.contains("map<string, int32> labels = 1;"));
        assert!(!text.contains("message LabelsEntry"));
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(safe_proto_path("../bad.proto").is_err());
        assert!(!is_safe_component(".."));
    }
}

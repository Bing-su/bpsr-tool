// Copyright 2025 PotRooms
// SPDX-License-Identifier: GPL-3.0-only

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::metadata::{Field, FieldKind};

pub(crate) struct Localization {
    values: HashMap<i32, String>,
}

impl Localization {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let count = reader.count()?;
        let mut indices = Vec::with_capacity(count);
        for _ in 0..count {
            indices.push((reader.i32()?, reader.i32()?));
        }
        let count = reader.count()?;
        let mut strings = Vec::with_capacity(count);
        for _ in 0..count {
            strings.push(reader.dotnet_string()?);
        }
        for _ in 0..2 {
            let count = reader.count()?;
            for _ in 0..count {
                reader.i32()?;
                reader.i32()?;
            }
        }
        let values = indices
            .into_iter()
            .filter_map(|(key, index)| {
                strings
                    .get(index as usize)
                    .cloned()
                    .map(|value| (key, value))
            })
            .collect();
        Ok(Self { values })
    }
    fn get(&self, key: i32) -> String {
        self.values.get(&key).cloned().unwrap_or_default()
    }
}

pub(crate) fn parse(bytes: &[u8], fields: &[Field], localization: &Localization) -> Result<Value> {
    let mut reader = Reader::new(bytes);
    reader.i64()?;
    let row_count = reader.count()?;
    let pool_count = reader.count()?;
    let data_offset = reader.count()?;
    let row_size = data_offset.checked_div(row_count).unwrap_or(0);
    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        rows.push(reader.i64()?);
    }
    let row_base = reader.position;
    reader.seek(
        row_base
            .checked_add(data_offset)
            .context("table data offset overflow")?,
    )?;
    let mut pools = HashMap::new();
    for _ in 0..pool_count {
        let kind = reader.i32()?;
        let length = reader.count()?;
        let data = reader.bytes(length)?.to_vec();
        if kind != 0 {
            pools.insert(kind, data);
        }
    }
    let mut output = BTreeMap::new();
    for (index, key) in rows.into_iter().enumerate() {
        let start = row_base
            .checked_add(index * row_size)
            .context("row offset overflow")?;
        let mut row = Reader::at(bytes, start)?;
        let mut used = 0;
        let mut object = Map::new();
        for field in fields.iter().filter(|field| field.name != "Key") {
            let size = field.kind.inline_size();
            if used + size > row_size {
                break;
            }
            used += size;
            object.insert(
                field.name.clone(),
                read_field(&mut row, &field.kind, &pools, localization)?,
            );
        }
        output.insert(key.to_string(), Value::Object(object));
    }
    Ok(serde_json::to_value(output)?)
}

fn read_field(
    reader: &mut Reader<'_>,
    kind: &FieldKind,
    pools: &HashMap<i32, Vec<u8>>,
    loc: &Localization,
) -> Result<Value> {
    Ok(match kind {
        FieldKind::I32 => json!(reader.i32()?),
        FieldKind::I64 => json!(reader.i64()?),
        FieldKind::Bool => json!(reader.u8()? != 0),
        FieldKind::F32 => json!(reader.f32()?),
        FieldKind::String => json!(read_string(pool(pools, 6), reader.i32()?, loc)),
        FieldKind::Vector2 => json!({"x": reader.f32()?, "y": reader.f32()?}),
        FieldKind::Vector3 => json!({"x": reader.f32()?, "y": reader.f32()?, "z": reader.f32()?}),
        FieldKind::Int32Array => json!(read_array::<4, _>(
            pool(pools, 1),
            reader.i32()?,
            i32::from_le_bytes
        )),
        FieldKind::Int64Array => json!(read_array::<8, _>(
            pool(pools, 2),
            reader.i32()?,
            i64::from_le_bytes
        )),
        FieldKind::NumberArray => json!(read_array::<4, _>(
            pool(pools, 3),
            reader.i32()?,
            f32::from_le_bytes
        )),
        FieldKind::Vector2Array => json!(read_vectors(pool(pools, 7), reader.i32()?, 2)),
        FieldKind::Vector3Array => json!(read_vectors(pool(pools, 8), reader.i32()?, 3)),
        FieldKind::StringArray => json!(read_string_array(
            pool(pools, 1),
            pool(pools, 6),
            reader.i32()?,
            loc,
            false
        )),
        FieldKind::MlStringArray => json!(read_string_array(
            pool(pools, 1),
            &[],
            reader.i32()?,
            loc,
            true
        )),
        FieldKind::Int32Table => json!(read_nested_i32(pool(pools, 1), reader.i32()?)),
        FieldKind::StringTable => json!(read_nested_strings(
            pool(pools, 1),
            pool(pools, 6),
            reader.i32()?,
            loc,
            false
        )),
        FieldKind::StringTripleArray => json!(read_string_triples(
            pool(pools, 1),
            pool(pools, 6),
            reader.i32()?,
            loc
        )),
        FieldKind::MlStringTable => json!(read_nested_strings(
            pool(pools, 1),
            &[],
            reader.i32()?,
            loc,
            true
        )),
        FieldKind::NumberTable => json!(read_number_table(
            pool(pools, 1),
            pool(pools, 3),
            reader.i32()?
        )),
        FieldKind::KvIntInt => json!(read_int_map(pool(pools, 4), reader.i32()?)),
        FieldKind::Other => json!(reader.i32()?),
    })
}

fn pool(pools: &HashMap<i32, Vec<u8>>, kind: i32) -> &[u8] {
    pools.get(&kind).map(Vec::as_slice).unwrap_or_default()
}

fn read_string(pool: &[u8], index: i32, loc: &Localization) -> String {
    let Ok(index) = usize::try_from(index) else {
        return String::new();
    };
    let Some(length) = pool
        .get(index..index + 2)
        .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
    else {
        return loc.get(index as i32);
    };
    let Ok(length) = usize::try_from(length) else {
        return String::new();
    };
    std::str::from_utf8(pool.get(index + 2..index + 2 + length).unwrap_or_default())
        .unwrap_or_default()
        .to_owned()
}

fn read_array<const N: usize, T>(
    pool: &[u8],
    index: i32,
    convert: impl Fn([u8; N]) -> T,
) -> Vec<T> {
    if index <= 0 {
        return vec![];
    }
    let index = index as usize;
    let count = pool
        .get(index..index + 2)
        .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
        .unwrap_or(0)
        .max(0) as usize;
    (0..count)
        .filter_map(|i| pool.get(index + 2 + i * N..index + 2 + (i + 1) * N))
        .map(|v| convert(v.try_into().unwrap()))
        .collect()
}

fn read_vectors(pool: &[u8], index: i32, dimensions: usize) -> Vec<Value> {
    if index <= 0 {
        return vec![];
    }
    let index = index as usize;
    let count = pool
        .get(index..index + 2)
        .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
        .unwrap_or(0)
        .max(0) as usize;
    (0..count)
        .filter_map(|i| {
            let start = index + 2 + i * dimensions * 4;
            let values: Vec<_> = (0..dimensions)
                .map(|d| {
                    pool.get(start + d * 4..start + (d + 1) * 4)
                        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                })
                .collect::<Option<_>>()?;
            Some(if dimensions == 2 {
                json!({"x":values[0],"y":values[1]})
            } else {
                json!({"x":values[0],"y":values[1],"z":values[2]})
            })
        })
        .collect()
}

fn read_string_array(
    ints: &[u8],
    strings: &[u8],
    index: i32,
    loc: &Localization,
    localized: bool,
) -> Vec<String> {
    read_array::<4, _>(ints, index, i32::from_le_bytes)
        .into_iter()
        .map(|v| {
            if localized {
                loc.get(v)
            } else {
                read_string(strings, v, loc)
            }
        })
        .collect()
}
fn read_nested_i32(pool: &[u8], index: i32) -> Vec<Vec<i32>> {
    read_array::<4, _>(pool, index, i32::from_le_bytes)
        .into_iter()
        .map(|v| read_array::<4, _>(pool, v, i32::from_le_bytes))
        .collect()
}
fn read_nested_strings(
    ints: &[u8],
    strings: &[u8],
    index: i32,
    loc: &Localization,
    localized: bool,
) -> Vec<Vec<String>> {
    read_array::<4, _>(ints, index, i32::from_le_bytes)
        .into_iter()
        .map(|v| read_string_array(ints, strings, v, loc, localized))
        .collect()
}
fn read_number_table(ints: &[u8], numbers: &[u8], index: i32) -> Vec<Vec<f32>> {
    read_array::<4, _>(ints, index, i32::from_le_bytes)
        .into_iter()
        .map(|v| read_array::<4, _>(numbers, v, f32::from_le_bytes))
        .collect()
}
fn read_string_triples(
    ints: &[u8],
    strings: &[u8],
    index: i32,
    loc: &Localization,
) -> Vec<Vec<String>> {
    read_array::<4, _>(ints, index, i32::from_le_bytes)
        .into_iter()
        .flat_map(|outer| {
            read_array::<4, _>(ints, outer, i32::from_le_bytes)
                .into_iter()
                .filter(|inner| *inner > 0)
                .map(|inner| read_string_array(ints, strings, inner, loc, false))
                .collect::<Vec<_>>()
        })
        .collect()
}
fn read_int_map(pool: &[u8], index: i32) -> BTreeMap<i32, i32> {
    if index <= 0 {
        return BTreeMap::new();
    }
    let index = index as usize;
    let count = pool
        .get(index..index + 2)
        .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
        .unwrap_or(0)
        .max(0) as usize;
    (0..count)
        .filter_map(|i| {
            let key = pool.get(index + 2 + i * 4..index + 6 + i * 4)?;
            let value = pool.get(index + 6 + i * 4..index + 10 + i * 4)?;
            Some((
                i32::from_le_bytes(key.try_into().unwrap()),
                i32::from_le_bytes(value.try_into().unwrap()),
            ))
        })
        .collect()
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn at(bytes: &'a [u8], position: usize) -> Result<Self> {
        let mut r = Self::new(bytes);
        r.seek(position)?;
        Ok(r)
    }
    fn bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .context("offset overflow")?;
        let value = self
            .bytes
            .get(self.position..end)
            .context("truncated data")?;
        self.position = end;
        Ok(value)
    }
    fn seek(&mut self, position: usize) -> Result<()> {
        if position > self.bytes.len() {
            bail!("offset outside data");
        }
        self.position = position;
        Ok(())
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize> {
        usize::try_from(self.i32()?).context("negative count")
    }
    fn dotnet_string(&mut self) -> Result<String> {
        let length = self.seven_bit()?;
        Ok(std::str::from_utf8(self.bytes(length)?)?.to_owned())
    }
    fn seven_bit(&mut self) -> Result<usize> {
        let mut result = 0usize;
        for shift in (0..35).step_by(7) {
            let byte = self.u8()?;
            result |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
        }
        bail!("invalid 7-bit integer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_pool_array() {
        let pool = [0, 0, 2, 0, 3, 0, 0, 0, 5, 0, 0, 0];
        assert_eq!(read_array::<4, _>(&pool, 2, i32::from_le_bytes), [3, 5]);
    }

    #[test]
    fn parses_localization_and_table_row() {
        let localization = Localization::parse(&[0; 16]).unwrap();
        let mut table = Vec::new();
        table.extend(0_i64.to_le_bytes());
        table.extend(1_i32.to_le_bytes());
        table.extend(1_i32.to_le_bytes());
        table.extend(8_i32.to_le_bytes());
        table.extend(42_i64.to_le_bytes());
        table.extend(7_i32.to_le_bytes());
        table.extend(0_i32.to_le_bytes());
        table.extend(6_i32.to_le_bytes());
        table.extend(5_i32.to_le_bytes());
        table.extend([3, 0, b'a', b'b', b'c']);
        let fields = [
            Field {
                name: "Value".into(),
                kind: FieldKind::I32,
            },
            Field {
                name: "Name".into(),
                kind: FieldKind::String,
            },
        ];
        let value = parse(&table, &fields, &localization).unwrap();
        assert_eq!(value["42"]["Value"], 7);
        assert_eq!(value["42"]["Name"], "abc");
    }

    #[test]
    fn reads_int_map_like_original() {
        let pool = [0, 0, 2, 0, 10, 0, 0, 0, 20, 0, 0, 0, 30, 0, 0, 0];
        assert_eq!(read_int_map(&pool, 2), BTreeMap::from([(10, 20), (20, 30)]));
    }
}

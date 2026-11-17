//! Shared Apache Parquet row decoder for external datasets (`multilexnorm`, `memory_agent_bench`).

use std::fs::File;
use std::path::Path;

use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use serde_json::{Map, Number, Value};

use crate::LabError;

/// Converts a Parquet [`Field`] into a normalized [`serde_json::Value`].
#[must_use]
pub fn parquet_field_to_json(field: &Field) -> Value {
    match field {
        Field::Null => Value::Null,
        Field::Bool(b) => Value::Bool(*b),
        Field::Byte(v) => Value::Number(Number::from(*v)),
        Field::Short(v) => Value::Number(Number::from(*v)),
        Field::Int(v) => Value::Number(Number::from(*v)),
        Field::Long(v) => Value::Number(Number::from(*v)),
        Field::UByte(v) => Value::Number(Number::from(*v)),
        Field::UShort(v) => Value::Number(Number::from(*v)),
        Field::UInt(v) => Value::Number(Number::from(*v)),
        Field::ULong(v) => Value::Number(Number::from(*v)),
        Field::Float(v) => Number::from_f64(f64::from(*v)).map_or(Value::Null, Value::Number),
        Field::Double(v) => Number::from_f64(*v).map_or(Value::Null, Value::Number),
        Field::Str(s) => Value::String(s.clone()),
        Field::Bytes(b) => Value::String(String::from_utf8_lossy(b.data()).into_owned()),
        Field::Decimal(d) => Value::String(format!("{d:?}")),
        Field::Date(d) => Value::Number(Number::from(*d)),
        Field::TimestampMillis(ms) => Value::Number(Number::from(*ms)),
        Field::TimestampMicros(us) => Value::Number(Number::from(*us)),
        Field::TimeMillis(ms) => Value::Number(Number::from(*ms)),
        Field::TimeMicros(us) => Value::Number(Number::from(*us)),
        Field::Float16(h) => {
            Number::from_f64(f64::from(h.to_f32())).map_or(Value::Null, Value::Number)
        }
        Field::ListInternal(list) => {
            Value::Array(list.elements().iter().map(parquet_field_to_json).collect())
        }
        Field::MapInternal(map) => {
            let mut obj = Map::new();
            for (k, v) in map.entries() {
                let key = match k {
                    Field::Str(s) => s.clone(),
                    other => parquet_field_to_json(other).to_string(),
                };
                obj.insert(key, parquet_field_to_json(v));
            }
            Value::Object(obj)
        }
        Field::Group(row) => {
            let mut obj = Map::new();
            for (name, val) in row.get_column_iter() {
                obj.insert(name.clone(), parquet_field_to_json(val));
            }
            Value::Object(obj)
        }
    }
}

/// Reads all rows from a Parquet file into JSON objects keyed by column name.
pub fn read_parquet_rows(path: &Path) -> Result<Vec<Value>, LabError> {
    let file = File::open(path)?;
    let reader = SerializedFileReader::new(file)
        .map_err(|e| LabError::Serialization(format!("parquet open '{}': {e}", path.display())))?;
    let iter = reader
        .get_row_iter(None)
        .map_err(|e| LabError::Serialization(format!("parquet iter '{}': {e}", path.display())))?;

    let mut rows = Vec::new();
    for row_res in iter {
        let row = row_res.map_err(|e| {
            LabError::Serialization(format!("parquet row '{}': {e}", path.display()))
        })?;
        let mut obj = Map::new();
        for (name, val) in row.get_column_iter() {
            obj.insert(name.clone(), parquet_field_to_json(val));
        }
        rows.push(Value::Object(obj));
    }
    Ok(rows)
}

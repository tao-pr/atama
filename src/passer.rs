//! Passer: The implementation of the data capture or data passer

use std::collections::BTreeMap;

/// Where a value comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Passer {
    FromFile(String),
    /// Data written directly in the job file.
    Data(Value),
    FromEnvVar(String),
    FromStdoutOfCommand(String),
}

/// A hardcoded value that keeps its type.
///
/// The job file is parsed at run time, so the type of each value is only known
/// then. A closed enum keeps `Clone`, `Debug`, and `PartialEq`, which
/// `Box<dyn Any>` loses, and lets callers `match` on the type.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<Value>),
    /// Keys are sorted, so output is stable.
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// Name of the value's type, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Returns ints as floats too.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_owned())
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(items: Vec<T>) -> Self {
        Value::List(items.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(opt: Option<T>) -> Self {
        opt.map_or(Value::Null, Into::into)
    }
}

impl<K: Into<String>, V: Into<Value>> FromIterator<(K, V)> for Value {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Value::Map(iter.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_type_of_each_value() {
        let data = Passer::Data(Value::from(vec![
            Value::from(1),
            Value::from(2.5),
            Value::from("three"),
            Value::from(true),
            Value::from(None::<i64>),
        ]));
        let Passer::Data(Value::List(items)) = data else {
            panic!("expected a list");
        };
        let types: Vec<_> = items.iter().map(Value::type_name).collect();
        assert_eq!(types, ["int", "float", "string", "bool", "null"]);
    }

    #[test]
    fn builds_map_from_pairs() {
        let value: Value = [("name", Value::from("atama")), ("retries", Value::from(3))]
            .into_iter()
            .collect();
        let map = value.as_map().unwrap();
        assert_eq!(map["name"].as_str(), Some("atama"));
        assert_eq!(map["retries"].as_int(), Some(3));
    }
}

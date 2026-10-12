//! Tests for the data passer and its typed values.

use atama::{Passer, Value};

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

use crate::js_binding::{properties::Properties, value::Value};
use crate::serialize::err::{Error, Result};
use anyhow::anyhow;
use serde::de::{self, Error as SerError};
use serde::forward_to_deserialize_any;

use super::sanitize_key;

impl SerError for Error {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Error::Custom(anyhow!(msg.to_string()))
    }
}

enum DeserializerValue {
    Value(Value),
    MapKey(String),
}

pub struct Deserializer {
    value: DeserializerValue,
}

impl From<Value> for Deserializer {
    fn from(value: Value) -> Self {
        let value = DeserializerValue::Value(value);
        Self { value }
    }
}

impl<'de, 'a> de::Deserializer<'de> for &'a mut Deserializer {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match &self.value {
            DeserializerValue::MapKey(key) => visitor.visit_str(key.as_str()),
            DeserializerValue::Value(value) => {
                if value.is_repr_as_i32() {
                    return visitor.visit_i32(value.as_i32());
                }

                if value.is_repr_as_f64() {
                    let val = value.as_f64()?;
                    return visitor.visit_f64(val);
                }

                if value.is_bool() {
                    let val = value.as_bool()?;
                    return visitor.visit_bool(val);
                }

                if value.is_null_or_undefined() {
                    return visitor.visit_unit();
                }

                if value.is_str() {
                    let val = value.as_str()?;
                    return visitor.visit_str(&val);
                }

                if value.is_array() {
                    let val = value.get_property("length")?;
                    let length = val.inner() as u32;
                    let seq = value.clone();
                    let seq_access = SeqAccess {
                        de: self,
                        length,
                        seq,
                        index: 0,
                    };
                    return visitor.visit_seq(seq_access);
                }

                if value.is_object() {
                    let properties = value.properties()?;
                    let map_access = MapAccess {
                        de: self,
                        properties,
                    };
                    return visitor.visit_map(map_access);
                }

                Err(Error::Custom(anyhow!(
                    "Couldn't deserialize value: {:?}",
                    value
                )))
            }
        }
    }

    fn is_human_readable(&self) -> bool {
        false
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match &self.value {
            DeserializerValue::MapKey(_key) => {
                unreachable!()
            }
            DeserializerValue::Value(value) => {
                if value.is_null_or_undefined() {
                    visitor.visit_none()
                } else {
                    visitor.visit_some(self)
                }
            }
        }
    }

    fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        unimplemented!()
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

struct MapAccess<'a> {
    de: &'a mut Deserializer,
    properties: Properties,
}

impl<'a, 'de> de::MapAccess<'de> for MapAccess<'a> {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where
        K: de::DeserializeSeed<'de>,
    {
        if let Some(key) = self.properties.next_key()? {
            let key = sanitize_key(&key, convert_case::Case::Snake)?;
            self.de.value = DeserializerValue::MapKey(key);
            seed.deserialize(&mut *self.de).map(Some)
        } else {
            Ok(None)
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
    where
        V: de::DeserializeSeed<'de>,
    {
        self.de.value = DeserializerValue::Value(self.properties.next_value()?);
        seed.deserialize(&mut *self.de)
    }
}

struct SeqAccess<'a> {
    de: &'a mut Deserializer,
    seq: Value,
    length: u32,
    index: u32,
}

impl<'a, 'de> de::SeqAccess<'de> for SeqAccess<'a> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        if self.index < self.length {
            self.de.value = DeserializerValue::Value(self.seq.get_indexed_property(self.index)?);
            self.index += 1;
            seed.deserialize(&mut *self.de).map(Some)
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::Deserializer as ValueDeserializer;
    use crate::js_binding::context::Context;
    use crate::js_binding::value::Value;
    use serde::de::DeserializeOwned;

    fn deserialize_value<T>(v: Value) -> T
    where
        T: DeserializeOwned,
    {
        let mut deserializer = ValueDeserializer::from(v);
        T::deserialize(&mut deserializer).unwrap()
    }

    #[test]
    fn test_null() {
        let context = Context::default();
        let val = context.null_value().unwrap();
        let actual = deserialize_value::<()>(val);
        assert_eq!((), actual);
    }

    #[test]
    fn test_undefined() {
        let context = Context::default();
        let val = context.undefined_value().unwrap();
        let actual = deserialize_value::<()>(val);
        assert_eq!((), actual);
    }

    #[test]
    fn test_nan() {
        let context = Context::default();
        let val = context.value_from_f64(f64::NAN).unwrap();
        let actual = deserialize_value::<f64>(val);
        assert!(actual.is_nan());
    }

    #[test]
    fn test_infinity() {
        let context = Context::default();
        let val = context.value_from_f64(f64::INFINITY).unwrap();
        let actual = deserialize_value::<f64>(val);
        assert!(actual.is_infinite() && actual.is_sign_positive());
    }

    #[test]
    fn test_negative_infinity() {
        let context = Context::default();
        let val = context.value_from_f64(f64::NEG_INFINITY).unwrap();
        let actual = deserialize_value::<f64>(val);
        assert!(actual.is_infinite() && actual.is_sign_negative());
    }

    #[test]
    fn test_map_always_converts_keys_to_string() {
        // Sanity check to make sure the quickjs VM always store object
        // object keys as a string an not a numerical value.
        let context = Context::default();
        context.eval_global("main", "var a = {1337: 42};").unwrap();
        let val = context.global_object().unwrap().get_property("a").unwrap();

        let actual = deserialize_value::<BTreeMap<String, i32>>(val);

        assert_eq!(42, *actual.get("1337").unwrap())
    }

    #[test]
    #[should_panic]
    fn test_map_does_not_support_non_string_keys() {
        // Sanity check to make sure it's not possible to deserialize
        // to a map where keys are not strings (e.g. numerical value).
        let context = Context::default();
        context.eval_global("main", "var a = {1337: 42};").unwrap();
        let val = context.global_object().unwrap().get_property("a").unwrap();

        deserialize_value::<BTreeMap<i32, i32>>(val);
    }

    #[test]
    fn test_map_keys_are_converted_to_snake_case() {
        let context = Context::default();
        let val = context.object_value().unwrap();
        val.set_property("hello_wold", context.value_from_i32(1).unwrap())
            .unwrap();
        val.set_property("toto", context.value_from_i32(2).unwrap())
            .unwrap();
        val.set_property("fooBar", context.value_from_i32(3).unwrap())
            .unwrap();
        val.set_property("Joyeux Noël", context.value_from_i32(4).unwrap())
            .unwrap();
        val.set_property("kebab-case", context.value_from_i32(5).unwrap())
            .unwrap();

        let actual = deserialize_value::<BTreeMap<String, i32>>(val);

        assert_eq!(1, *actual.get("hello_wold").unwrap());
        assert_eq!(2, *actual.get("toto").unwrap());
        assert_eq!(3, *actual.get("foo_bar").unwrap());
        assert_eq!(4, *actual.get("joyeux_noël").unwrap());
        assert_eq!(5, *actual.get("kebab_case").unwrap());
    }
}

//! In-memory storage for host checks, using the firmware's real key definitions.
#![allow(dead_code)]
use std::sync::RwLock;

pub struct KvsKey<T> {
    default: Option<T>,
    value: RwLock<Option<T>>,
}

impl<T: Clone> KvsKey<T> {
    const fn new(name: &'static str) -> Self {
        assert!(name.len() < 16);
        Self {
            default: None,
            value: RwLock::new(None),
        }
    }

    const fn new_with_default(name: &'static str, default: T) -> Self {
        assert!(name.len() < 16);
        Self {
            default: Some(default),
            value: RwLock::new(None),
        }
    }

    pub fn get(&self) -> Option<T> {
        self.value
            .read()
            .unwrap()
            .as_ref()
            .or(self.default.as_ref())
            .cloned()
    }

    pub fn set(&self, value: &T) {
        *self.value.write().unwrap() = Some(value.clone());
    }

    pub fn flush(&self) {}
}

#[path = "../src/kvs/keys.rs"]
pub mod keys;

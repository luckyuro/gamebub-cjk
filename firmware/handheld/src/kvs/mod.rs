use embedded_svc::storage::RawStorage;
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsCustom};
use serde::{de::DeserializeOwned, Serialize};
use smallvec::SmallVec;
use std::sync::{Mutex, MutexGuard, OnceLock, RwLock};

pub mod keys;

static KVS: OnceLock<Mutex<Kvs>> = OnceLock::new();
static NAMESPACE: &'static str = "gb";

/// TODO: a few possible changes:
///  * Use a single global lock for all keys and all keys' caches?
///  * use a macro when defining keys to ensure they're all in flush_all
pub struct Kvs {
    nvs_main: EspNvs<NvsCustom>,
}

impl Kvs {
    pub fn init() -> Result<(), anyhow::Error> {
        let nvs_main = EspNvs::new(EspNvsPartition::<NvsCustom>::take("nvs")?, NAMESPACE, true)?;

        let kvs = Kvs { nvs_main };
        KVS.set(Mutex::new(kvs))
            .map_err(|_| ())
            .expect("KVS already initialized");

        Ok(())
    }

    pub fn get() -> MutexGuard<'static, Kvs> {
        KVS.get().unwrap().lock().unwrap()
    }

    fn nvs(&mut self, read_only: bool) -> &mut EspNvs<NvsCustom> {
        if read_only {
            panic!("nvs_ro not used");
        } else {
            &mut self.nvs_main
        }
    }
}

const SMALL_SIZE: usize = 128;
const MAX_VALUE_SIZE: usize = 4096;

pub struct CacheEntry<T> {
    value: Option<T>,
    dirty: bool,
}

pub struct KvsKey<T> {
    name: &'static str,
    read_only: bool,
    default: Option<T>,
    cache: RwLock<Option<CacheEntry<T>>>,
}

impl<T: Serialize + DeserializeOwned + Clone> KvsKey<T> {
    const fn new(name: &'static str) -> Self {
        assert!(name.len() < 16);
        KvsKey::<T> {
            name,
            read_only: false,
            default: None,
            cache: RwLock::new(None),
        }
    }

    const fn new_with_default(name: &'static str, default: T) -> Self {
        assert!(name.len() < 16);
        KvsKey::<T> {
            name,
            read_only: false,
            default: Some(default),
            cache: RwLock::new(None),
        }
    }

    /// Get the value directly from NVS, without going through the cache.
    fn get_direct(&self) -> Option<T> {
        let mut kvs = Kvs::get();
        let nvs = kvs.nvs(self.read_only);
        let len = match nvs.len(self.name) {
            Ok(Some(len)) => len,
            Ok(None) => return None,
            Err(error) => {
                log::error!("Failed to read KVS length for {}: {error}", self.name);
                return None;
            }
        };
        if len > MAX_VALUE_SIZE {
            log::error!(
                "Ignoring oversized KVS value for {}: {} bytes",
                self.name,
                len
            );
            return None;
        }
        let mut v = SmallVec::<[u8; SMALL_SIZE]>::from_elem(0, len);
        let data = match nvs.get_raw(self.name, &mut v) {
            Ok(Some(data)) => data,
            Ok(None) => return None,
            Err(error) => {
                log::error!("Failed to read KVS value for {}: {error}", self.name);
                return None;
            }
        };
        match postcard::from_bytes(data) {
            Ok(value) => Some(value),
            Err(error) => {
                log::error!("Ignoring invalid KVS value for {}: {error}", self.name);
                None
            }
        }
    }

    pub fn get(&self) -> Option<T> {
        // If it's in the cache, return it.
        let cache = self.cache.read().unwrap();
        if let Some(value) = cache.as_ref() {
            return value.value.as_ref().or(self.default.as_ref()).cloned();
        }

        // Otherwise, fetch from storage.
        let value = self.get_direct();

        // Reload the cache.
        std::mem::drop(cache);
        let mut cache = self.cache.write().unwrap();
        *cache = Some(CacheEntry {
            value: value.clone(),
            dirty: false,
        });

        value.or_else(|| self.default.clone())
    }

    fn set_direct(&self, value: &T) -> bool {
        let mut kvs = Kvs::get();
        let nvs = kvs.nvs(self.read_only);
        let mut v = SmallVec::<[u8; SMALL_SIZE]>::new();
        if let Err(error) = postcard::to_io(value, &mut v) {
            log::error!("Failed to serialize KVS value for {}: {error}", self.name);
            return false;
        }
        if let Err(error) = nvs.set_raw(self.name, &v) {
            log::error!("Failed to write KVS value for {}: {error}", self.name);
            return false;
        }
        true
    }

    pub fn set(&self, value: &T) {
        let mut cache = self.cache.write().unwrap();
        *cache = Some(CacheEntry {
            value: Some(value.clone()),
            dirty: true,
        });
    }

    pub fn flush(&self) {
        let mut cache = self.cache.write().unwrap();
        if let Some(cache) = cache.as_mut() {
            if cache.dirty {
                if let Some(value) = cache.value.as_ref() {
                    log::info!("Flushing KVS: {}", self.name);
                    if self.set_direct(value) {
                        cache.dirty = false;
                    }
                } else {
                    cache.dirty = false;
                }
            }
        }
    }
}

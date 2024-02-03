use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use crate::{DataInitError, FoRegistry};

#[derive(Debug, Serialize)]
pub(crate) struct FoRegistryCache<T>(FoRegistryCacheHeader, T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FoRegistryCache<Result<T, DataInitError>> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor<T>(PhantomData<fn() -> T>);

        impl<'vi, T: serde::Deserialize<'vi>> serde::de::Visitor<'vi> for Visitor<T> {
            type Value = FoRegistryCache<Result<T, DataInitError>>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Expecting FoRegistryCache")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'vi>,
            {
                let header: FoRegistryCacheHeader = seq
                    .next_element()?
                    .ok_or(serde::de::Error::missing_field("header"))?;
                let data = if &header.pattern != b"FoRegistry"
                    || header.version != FoRegistry::version()
                {
                    Err(DataInitError::CacheIncompatible)
                } else {
                    Ok(seq
                        .next_element()?
                        .ok_or(serde::de::Error::missing_field("data"))?)
                };
                Ok(FoRegistryCache(header, data))
            }
        }
        deserializer.deserialize_tuple_struct("FoRegistryCache", 2, Visitor(PhantomData))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FoRegistryCacheHeader {
    pattern: [u8; 10],
    version: u32,
}

impl<T> FoRegistryCache<T> {
    pub(crate) fn into_data(self) -> T {
        self.1
    }
}

impl<'a> FoRegistryCache<&'a FoRegistry> {
    pub(crate) fn new(data: &'a FoRegistry) -> Self {
        FoRegistryCache(
            FoRegistryCacheHeader {
                pattern: *b"FoRegistry",
                version: FoRegistry::version(),
            },
            data,
        )
    }
}

// SPDX-License-Identifier: MIT

use serde::de::{self, Deserializer, Visitor};
use serde::Deserialize;
use std::convert::TryFrom;
use std::fmt;

struct Defaults;

impl Defaults {
    #[inline]
    const fn unlock() -> bool {
        true
    }

    #[inline]
    const fn unlock_migration() -> bool {
        false
    }
}

pub(crate) fn deserialize_optional_pci_id<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionalPciIdVisitor;

    impl<'de> Visitor<'de> for OptionalPciIdVisitor {
        type Value = Option<u32>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an integer or a string like '0x2230'")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserialize_pci_id(deserializer).map(Some)
        }
    }

    deserializer.deserialize_option(OptionalPciIdVisitor)
}

fn deserialize_pci_id<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    struct PciIdVisitor;

    impl<'de> Visitor<'de> for PciIdVisitor {
        type Value = u32;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a u32 integer or a string like '0x2230'")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u32::try_from(value).map_err(|_| E::custom("PCI ID must fit into u32"))
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let value = u64::try_from(value).map_err(|_| E::custom("PCI ID cannot be negative"))?;
            self.visit_u64(value)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let value = value.trim();
            let (radix, digits) = match value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
            {
                Some(hex) => (16, hex),
                None => (10, value),
            };

            u32::from_str_radix(digits, radix)
                .map_err(|_| E::custom(format!("invalid PCI ID '{value}'")))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(PciIdVisitor)
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "Defaults::unlock")]
    pub unlock: bool,
    #[serde(default = "Defaults::unlock_migration")]
    pub unlock_migration: bool,
    #[serde(default, deserialize_with = "deserialize_optional_pci_id")]
    pub spoofed_devid: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_pci_id")]
    pub spoofed_subsysid: Option<u32>,
}

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self {
            unlock: Defaults::unlock(),
            unlock_migration: Defaults::unlock_migration(),
            spoofed_devid: None,
            spoofed_subsysid: None,
        }
    }
}

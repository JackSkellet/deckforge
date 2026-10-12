//! Device families from Elgato's HID reference and SDK DeviceType enumeration.
//! https://docs.elgato.com/streamdeck/hid/ and /streamdeck/sdk/guides/devices/
//! Product aliases share geometry/protocol; they do not imply physical testing.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    Mini,
    Classic,
    Xl,
    #[default]
    Plus,
    PlusXl,
    Neo,
    Pedal,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Capabilities {
    pub transport_supported: bool,
    pub has_lcd: bool,
    pub key_images: bool,
    pub brightness: bool,
    pub sleep: bool,
    pub full_lcd: bool,
    pub strip: bool,
    pub touchscreen: bool,
    pub sensors: u8,
}
impl Model {
    pub const fn all() -> &'static [Self] {
        &[
            Self::Mini,
            Self::Classic,
            Self::Xl,
            Self::Plus,
            Self::PlusXl,
            Self::Neo,
            Self::Pedal,
        ]
    }
    pub const fn from_product(pid: u16) -> Option<Self> {
        match pid {
            0x0063 | 0x0090 | 0x00b3 | 0x00b8 => Some(Self::Mini),
            0x006d | 0x0080 | 0x00a5 | 0x00b9 => Some(Self::Classic),
            0x006c | 0x008f | 0x00ba => Some(Self::Xl),
            0x0084 => Some(Self::Plus),
            0x00c6 => Some(Self::PlusXl),
            0x009a => Some(Self::Neo),
            // Original Windows7.6 registry0x140135a65 and Pedal factory/vtable.
            0x0086 => Some(Self::Pedal),
            // Legacy2017 has its own protocol; image transformations and version
            // dependencies remain unresolved, so never open it as Classic2019.
            _ => None,
        }
    }
    pub fn from_profile_model(model: &str) -> Option<Self> {
        match model {
            "20GAI9901" | "20GAI9902" | "mini" => Some(Self::Mini),
            "20GAA9901" | "20GAA9902" | "20GBA9901" | "20GBL9901" | "classic" => {
                Some(Self::Classic)
            }
            "20GAT9901" | "20GAT9902" | "xl" => Some(Self::Xl),
            "20GBD9901" | "plus" => Some(Self::Plus),
            "20GBX9901" | "plus-xl" => Some(Self::PlusXl),
            "20GBJ9901" | "neo" => Some(Self::Neo),
            "20GBF9901" | "deckforge-pedal" | "pedal" => Some(Self::Pedal),
            _ => None,
        }
    }
    pub const fn id(self) -> &'static str {
        match self {
            Self::Mini => "mini",
            Self::Classic => "classic",
            Self::Xl => "xl",
            Self::Plus => "plus",
            Self::PlusXl => "plus-xl",
            Self::Neo => "neo",
            Self::Pedal => "pedal",
        }
    }
    pub const fn sensor_count(self) -> u8 {
        if matches!(self, Self::Neo) { 2 } else { 0 }
    }
    pub const fn has_lcd(self) -> bool {
        !matches!(self, Self::Pedal)
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mini => "Stream Deck Mini",
            Self::Classic => "Stream Deck",
            Self::Xl => "Stream Deck XL",
            Self::Plus => "Stream Deck +",
            Self::PlusXl => "Stream Deck + XL",
            Self::Neo => "Stream Deck Neo",
            Self::Pedal => "Stream Deck Pedal",
        }
    }
    pub const fn profile_model(self) -> &'static str {
        match self {
            Self::Mini => "20GAI9901",
            Self::Classic => "20GBA9901",
            Self::Xl => "20GAT9901",
            Self::Plus => "20GBD9901",
            Self::Neo => "20GBJ9901",
            // Plus XL identity recovered from the shipped 7.6 profile; its HID
            // page currently repeats Plus's identifier. Pedal is from native RTTI
            // and its registry/factory binding in the same shipped executable.
            Self::PlusXl => "20GBX9901",
            Self::Pedal => "20GBF9901",
        }
    }
    pub const fn sdk_type(self) -> u8 {
        match self {
            Self::Mini => 1,
            Self::Classic => 0,
            Self::Xl => 2,
            Self::Plus => 7,
            Self::PlusXl => 13,
            Self::Neo => 9,
            Self::Pedal => 5,
        }
    }
    pub const fn columns(self) -> u8 {
        match self {
            Self::Mini | Self::Pedal => 3,
            Self::Classic => 5,
            Self::Xl => 8,
            Self::Plus | Self::Neo => 4,
            Self::PlusXl => 9,
        }
    }
    pub const fn rows(self) -> u8 {
        match self {
            Self::Mini | Self::Plus | Self::Neo => 2,
            Self::Classic => 3,
            Self::Xl | Self::PlusXl => 4,
            Self::Pedal => 1,
        }
    }
    pub const fn key_count(self) -> u8 {
        self.columns() * self.rows()
    }
    pub const fn dial_count(self) -> u8 {
        match self {
            Self::Plus => 4,
            Self::PlusXl => 6,
            _ => 0,
        }
    }
    pub const fn key_width(self) -> u32 {
        match self {
            Self::Mini => 80,
            Self::Classic => 72,
            Self::Xl | Self::Neo => 96,
            Self::Plus => 120,
            Self::PlusXl => 112,
            Self::Pedal => 0,
        }
    }
    pub const fn key_height(self) -> u32 {
        self.key_width()
    }
    pub const fn strip_width(self) -> u32 {
        match self {
            Self::Plus => 800,
            Self::PlusXl => 1200,
            Self::Neo => 248,
            _ => 0,
        }
    }
    pub const fn strip_height(self) -> u32 {
        match self {
            Self::Plus | Self::PlusXl => 100,
            Self::Neo => 58,
            _ => 0,
        }
    }
    pub const fn lcd_width(self) -> u32 {
        match self {
            Self::Mini => 320,
            Self::Classic | Self::Neo => 480,
            Self::Xl => 1024,
            Self::Plus => 800,
            Self::PlusXl => 1280,
            Self::Pedal => 0,
        }
    }
    pub const fn lcd_height(self) -> u32 {
        match self {
            Self::Mini => 240,
            Self::Classic => 272,
            Self::Neo => 320,
            Self::Xl => 600,
            Self::Plus => 480,
            Self::PlusXl => 800,
            Self::Pedal => 0,
        }
    }
    pub const fn capabilities(self) -> Capabilities {
        Capabilities {
            transport_supported: true,
            has_lcd: self.has_lcd(),
            key_images: !matches!(self, Self::Pedal),
            brightness: !matches!(self, Self::Pedal),
            sleep: !matches!(self, Self::Pedal),
            full_lcd: !matches!(self, Self::Mini | Self::Pedal),
            strip: self.strip_width() > 0,
            touchscreen: self.dial_count() > 0,
            sensors: if matches!(self, Self::Neo) { 2 } else { 0 },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documented_aliases_and_explicit_unknown_protocols() {
        for (pids, model) in [
            (&[0x63, 0x90, 0xb3, 0xb8][..], Model::Mini),
            (&[0x6d, 0x80, 0xa5, 0xb9][..], Model::Classic),
            (&[0x6c, 0x8f, 0xba][..], Model::Xl),
        ] {
            for &pid in pids {
                assert_eq!(Model::from_product(pid), Some(model));
            }
        }
        assert_eq!(Model::from_product(0x60), None);
        assert_eq!(Model::from_product(0x86), Some(Model::Pedal));
        assert!(Model::Pedal.capabilities().transport_supported);
        assert!(!Model::Pedal.capabilities().key_images);
        assert!(!Model::Pedal.capabilities().sleep);
        assert!(!Model::Pedal.capabilities().brightness);
    }
    #[test]
    fn geometry_and_serialization_roundtrip() {
        for &m in Model::all() {
            assert_eq!(m.key_count(), m.columns() * m.rows());
            assert_eq!(Model::from_profile_model(m.profile_model()), Some(m));
            assert_eq!(
                serde_json::from_value::<Model>(serde_json::to_value(m).unwrap()).unwrap(),
                m
            );
        }
        assert_eq!(Model::PlusXl.key_count(), 36);
        assert_eq!(Model::PlusXl.dial_count(), 6);
        assert_eq!(Model::Neo.capabilities().sensors, 2);
        assert!(!Model::Neo.capabilities().touchscreen);
    }
}

//! Device families from Elgato's HID reference and SDK DeviceType enumeration.
//! https://docs.elgato.com/streamdeck/hid/ and /streamdeck/sdk/guides/devices/
//! Product aliases share geometry/protocol; they do not imply physical testing.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    Mini,
    Classic,
    #[serde(rename = "classic-2017", alias = "classic2017")]
    Classic2017,
    Xl,
    #[default]
    Plus,
    PlusXl,
    Neo,
    Studio,
    Galleon,
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
            Self::Classic2017,
            Self::Xl,
            Self::Plus,
            Self::PlusXl,
            Self::Neo,
            Self::Studio,
            Self::Galleon,
            Self::Pedal,
        ]
    }
    pub const fn from_product(pid: u16) -> Option<Self> {
        match pid {
            0x0063 | 0x0090 | 0x00b3 | 0x00b8 => Some(Self::Mini),
            0x0060 => Some(Self::Classic2017),
            0x006d | 0x0080 | 0x00a5 | 0x00b9 => Some(Self::Classic),
            0x006c | 0x008f | 0x00ba => Some(Self::Xl),
            0x0084 => Some(Self::Plus),
            0x00c6 => Some(Self::PlusXl),
            0x009a => Some(Self::Neo),
            0x00aa => Some(Self::Studio),
            // Original Windows7.6 registry0x140135a65 and Pedal factory/vtable.
            0x0086 => Some(Self::Pedal),
            _ => None,
        }
    }
    pub fn from_profile_model(model: &str) -> Option<Self> {
        match model {
            "20GAI9901" | "20GAI9902" | "mini" => Some(Self::Mini),
            "20GAA9901" | "classic-2017" => Some(Self::Classic2017),
            "20GAA9902" | "20GBA9901" | "20GBL9901" | "classic" => Some(Self::Classic),
            "20GAT9901" | "20GAT9902" | "xl" => Some(Self::Xl),
            "20GBD9901" | "plus" => Some(Self::Plus),
            "20GBX9901" | "plus-xl" => Some(Self::PlusXl),
            "20GBJ9901" | "neo" => Some(Self::Neo),
            "20GBO9901" | "studio" => Some(Self::Studio),
            "GRETSCH" | "GRETSCH1G" | "galleon" => Some(Self::Galleon),
            "20GBF9901" | "deckforge-pedal" | "pedal" => Some(Self::Pedal),
            _ => None,
        }
    }
    pub const fn id(self) -> &'static str {
        match self {
            Self::Mini => "mini",
            Self::Classic => "classic",
            Self::Classic2017 => "classic-2017",
            Self::Xl => "xl",
            Self::Plus => "plus",
            Self::PlusXl => "plus-xl",
            Self::Neo => "neo",
            Self::Studio => "studio",
            Self::Galleon => "galleon",
            Self::Pedal => "pedal",
        }
    }
    /// Both 15-key generations use the same SDK/profile coordinates. Their HID
    /// protocols and capabilities remain distinct.
    pub fn profile_compatible(self, other: Self) -> bool {
        self == other
            || matches!(
                (self, other),
                (Self::Classic, Self::Classic2017) | (Self::Classic2017, Self::Classic)
            )
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
            Self::Classic2017 => "Stream Deck (2017)",
            Self::Xl => "Stream Deck XL",
            Self::Plus => "Stream Deck +",
            Self::PlusXl => "Stream Deck + XL",
            Self::Neo => "Stream Deck Neo",
            Self::Studio => "Stream Deck Studio",
            Self::Galleon => "Galleon 100 SD",
            Self::Pedal => "Stream Deck Pedal",
        }
    }
    pub const fn profile_model(self) -> &'static str {
        match self {
            Self::Mini => "20GAI9901",
            Self::Classic => "20GBA9901",
            Self::Classic2017 => "20GAA9901",
            Self::Xl => "20GAT9901",
            Self::Plus => "20GBD9901",
            Self::Neo => "20GBJ9901",
            Self::Studio => "20GBO9901",
            Self::Galleon => "GRETSCH",
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
            Self::Classic | Self::Classic2017 => 0,
            Self::Xl => 2,
            Self::Plus => 7,
            Self::PlusXl => 13,
            Self::Neo => 9,
            Self::Studio => 10,
            Self::Galleon => 12,
            Self::Pedal => 5,
        }
    }
    pub const fn columns(self) -> u8 {
        match self {
            Self::Mini | Self::Pedal | Self::Galleon => 3,
            Self::Classic | Self::Classic2017 => 5,
            Self::Xl => 8,
            Self::Plus | Self::Neo => 4,
            Self::PlusXl => 9,
            Self::Studio => 16,
        }
    }
    pub const fn rows(self) -> u8 {
        match self {
            Self::Mini | Self::Plus | Self::Neo | Self::Studio => 2,
            Self::Classic | Self::Classic2017 => 3,
            Self::Xl | Self::PlusXl | Self::Galleon => 4,
            Self::Pedal => 1,
        }
    }
    pub const fn key_count(self) -> u8 {
        self.columns() * self.rows()
    }
    pub const fn dial_count(self) -> u8 {
        match self {
            Self::Plus | Self::Galleon => 4,
            Self::PlusXl => 6,
            Self::Studio => 2,
            _ => 0,
        }
    }
    pub const fn key_width(self) -> u32 {
        match self {
            Self::Mini => 80,
            Self::Classic | Self::Classic2017 => 72,
            Self::Xl | Self::Neo => 96,
            Self::Plus => 120,
            Self::PlusXl => 112,
            Self::Studio => 144,
            Self::Galleon => 160,
            Self::Pedal => 0,
        }
    }
    /// Logical encoder action positions can outnumber the physical controls.
    pub const fn physical_dial_count(self) -> u8 {
        if matches!(self, Self::Galleon) {
            2
        } else {
            self.dial_count()
        }
    }
    pub const fn encoder_columns(self) -> u8 {
        if matches!(self, Self::Galleon) {
            2
        } else {
            self.dial_count()
        }
    }
    pub const fn encoder_rows(self) -> u8 {
        if matches!(self, Self::Galleon) {
            2
        } else if self.dial_count() > 0 {
            1
        } else {
            0
        }
    }
    pub fn encoder_coordinates(self, index: u8) -> (u8, u8) {
        let columns = self.encoder_columns().max(1);
        (index % columns, index / columns)
    }
    pub fn encoder_size(self) -> (u32, u32) {
        if self == Self::Galleon {
            return (320, 160);
        }
        (
            self.strip_width() / u32::from(self.encoder_columns()).max(1),
            self.strip_height() / u32::from(self.encoder_rows()).max(1),
        )
    }
    pub fn encoder_origin(self, index: u8) -> (u32, u32) {
        let (column, row) = self.encoder_coordinates(index);
        if self == Self::Galleon {
            // Canonical composer: 200x100 cells, 15px margins, 20x10 gaps;
            // its 450x240 canvas is scaled by 1.6 onto the 720x384 display.
            (24 + u32::from(column) * 352, 24 + u32::from(row) * 176)
        } else {
            let (width, height) = self.encoder_size();
            (u32::from(column) * width, u32::from(row) * height)
        }
    }
    pub const fn key_height(self) -> u32 {
        if matches!(self, Self::Studio) {
            112
        } else {
            self.key_width()
        }
    }
    pub const fn strip_width(self) -> u32 {
        match self {
            Self::Plus => 800,
            Self::PlusXl => 1200,
            Self::Neo => 248,
            Self::Galleon => 720,
            _ => 0,
        }
    }
    pub const fn strip_height(self) -> u32 {
        match self {
            Self::Plus | Self::PlusXl => 100,
            Self::Neo => 58,
            Self::Galleon => 384,
            _ => 0,
        }
    }
    pub const fn lcd_width(self) -> u32 {
        match self {
            Self::Mini => 320,
            Self::Classic | Self::Classic2017 | Self::Neo => 480,
            Self::Xl => 1024,
            Self::Plus => 800,
            Self::PlusXl => 1280,
            Self::Galleon => 720,
            Self::Pedal | Self::Studio => 0,
        }
    }
    pub const fn lcd_height(self) -> u32 {
        match self {
            Self::Mini => 240,
            Self::Classic | Self::Classic2017 => 272,
            Self::Neo => 320,
            Self::Xl => 600,
            Self::Plus => 480,
            Self::PlusXl => 800,
            Self::Galleon => 1280,
            Self::Pedal | Self::Studio => 0,
        }
    }
    pub const fn capabilities(self) -> Capabilities {
        Capabilities {
            transport_supported: true,
            has_lcd: self.has_lcd(),
            key_images: !matches!(self, Self::Pedal),
            brightness: !matches!(self, Self::Pedal),
            // Studio's native setter exists, but readback is unproven. This
            // capability currently promises getter-verified timer control.
            sleep: !matches!(
                self,
                Self::Pedal | Self::Classic2017 | Self::Studio | Self::Galleon
            ),
            full_lcd: !matches!(
                self,
                Self::Mini | Self::Pedal | Self::Classic2017 | Self::Studio
            ),
            strip: self.strip_width() > 0,
            touchscreen: matches!(self, Self::Plus | Self::PlusXl),
            sensors: if matches!(self, Self::Neo) { 2 } else { 0 },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn galleon_separates_logical_widgets_from_physical_dials() {
        let model = Model::Galleon;
        assert_eq!(
            (model.columns(), model.rows(), model.key_count()),
            (3, 4, 12)
        );
        assert_eq!(
            (
                model.encoder_columns(),
                model.encoder_rows(),
                model.dial_count(),
                model.physical_dial_count()
            ),
            (2, 2, 4, 2)
        );
        assert_eq!(model.encoder_coordinates(3), (1, 1));
        assert_eq!(model.encoder_size(), (320, 160));
        assert_eq!(model.encoder_origin(3), (376, 200));
        assert_eq!((model.key_width(), model.key_height()), (160, 160));
        assert_eq!((model.lcd_width(), model.lcd_height()), (720, 1280));
        assert_eq!(Model::from_profile_model("GRETSCH"), Some(model));
        assert_eq!(Model::from_profile_model("GRETSCH1G"), Some(model));
        assert_eq!(model.sdk_type(), 12);
        assert!(model.capabilities().transport_supported);
        assert!(!model.capabilities().touchscreen);
        assert!(!model.profile_compatible(Model::Plus));
    }
    #[test]
    fn studio_rectangular_keys_and_displayless_encoders_are_distinct_capabilities() {
        let m = Model::Studio;
        assert_eq!(Model::from_product(0xaa), Some(m));
        assert_eq!(Model::from_profile_model("20GBO9901"), Some(m));
        assert_eq!(
            (m.columns(), m.rows(), m.key_count(), m.dial_count()),
            (16, 2, 32, 2)
        );
        assert_eq!((m.key_width(), m.key_height()), (144, 112));
        assert_eq!(m.sdk_type(), 10);
        assert!(m.capabilities().key_images && m.capabilities().brightness);
        assert!(
            !m.capabilities().strip
                && !m.capabilities().touchscreen
                && !m.capabilities().full_lcd
                && !m.capabilities().sleep
        );
        assert_eq!(
            (
                m.strip_width(),
                m.strip_height(),
                m.lcd_width(),
                m.lcd_height()
            ),
            (0, 0, 0, 0)
        );
    }
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
        assert_eq!(Model::from_product(0x60), Some(Model::Classic2017));
        assert_eq!(Model::from_product(0x86), Some(Model::Pedal));
        assert!(Model::Pedal.capabilities().transport_supported);
        assert!(!Model::Pedal.capabilities().key_images);
        assert!(!Model::Pedal.capabilities().sleep);
        assert!(!Model::Pedal.capabilities().brightness);
    }
    #[test]
    fn geometry_and_serialization_roundtrip() {
        for &m in Model::all() {
            assert_eq!(serde_json::to_value(m).unwrap(), m.id());
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

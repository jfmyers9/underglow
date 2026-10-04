use crate::sdk::rgb::{DeviceInfo, DeviceType};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixCoord {
    pub row: u8,
    pub column: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Zone {
    Function,
    Alpha,
    Navigation,
    Arrows,
    System,
}

#[derive(Clone, Debug)]
pub struct KeyPosition {
    pub coord: MatrixCoord,
    pub x: f32,
    pub y: f32,
    pub zone: Zone,
}

#[derive(Clone, Debug)]
pub struct KeyboardLayout {
    pub name: &'static str,
    pub width: f32,
    pub height: f32,
    keys: Vec<KeyPosition>,
}

impl KeyboardLayout {
    pub fn for_device(info: &DeviceInfo) -> Self {
        if info.device_type == DeviceType::Keyboard80 && info.max_columns == 17 {
            Self::wooting_80he()
        } else {
            Self::matrix(info)
        }
    }

    pub fn keys(&self) -> &[KeyPosition] {
        &self.keys
    }

    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    pub fn summary(&self) -> String {
        let min_x = self
            .keys
            .iter()
            .map(|key| key.x)
            .fold(f32::INFINITY, f32::min);
        let max_x = self
            .keys
            .iter()
            .map(|key| key.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = self
            .keys
            .iter()
            .map(|key| key.y)
            .fold(f32::INFINITY, f32::min);
        let max_y = self
            .keys
            .iter()
            .map(|key| key.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let mut counts = [
            (Zone::Function, 0),
            (Zone::Alpha, 0),
            (Zone::Navigation, 0),
            (Zone::Arrows, 0),
            (Zone::System, 0),
        ];
        for key in &self.keys {
            if let Some((_, count)) = counts.iter_mut().find(|(zone, _)| *zone == key.zone) {
                *count += 1;
            }
        }

        let zones = counts
            .iter()
            .filter(|(_, count)| *count > 0)
            .map(|(zone, count)| format!("{zone:?}: {count}"))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "{}: {:.1} x {:.1}, {} keys ({})",
            self.name,
            self.width,
            self.height,
            self.key_count(),
            zones
        ) + &format!(" bounds x:{min_x:.1}-{max_x:.1} y:{min_y:.1}-{max_y:.1}")
    }

    fn matrix(info: &DeviceInfo) -> Self {
        let mut keys = Vec::new();
        for row in 0..info.max_rows {
            for column in 0..info.max_columns {
                keys.push(KeyPosition {
                    coord: MatrixCoord { row, column },
                    x: f32::from(column),
                    y: f32::from(row),
                    zone: Zone::Alpha,
                });
            }
        }

        Self {
            name: "matrix",
            width: f32::from(info.max_columns.saturating_sub(1)).max(1.0),
            height: f32::from(info.max_rows.saturating_sub(1)).max(1.0),
            keys,
        }
    }

    fn wooting_80he() -> Self {
        let keys = wooting_signals::ripple::wooting_80he_geometry()
            .into_iter()
            .map(|key| KeyPosition {
                coord: MatrixCoord {
                    row: key.row,
                    column: key.column,
                },
                x: key.x,
                y: key.y,
                zone: zone_for_80he(key.row, key.column),
            })
            .collect();

        Self {
            name: "wooting-80he",
            width: 17.0,
            height: 5.0,
            keys,
        }
    }
}

fn zone_for_80he(row: u8, column: u8) -> Zone {
    match (row, column) {
        (0, _) => Zone::Function,
        (5, 13..=16) => Zone::Arrows,
        (_, 16) => Zone::System,
        (_, 14..=15) => Zone::Navigation,
        _ => Zone::Alpha,
    }
}

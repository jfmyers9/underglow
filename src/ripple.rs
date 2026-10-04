//! Pure ripple simulation shared by the hardware engine and interactive previews.
//! No SDK, windowing, wall-clock, or device access occurs here.

/// Stable keyboard matrix address, independent of OS text layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatrixCoord {
    pub row: u8,
    pub column: u8,
}

/// A key center in keyboard units and its matrix address.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyGeometry {
    pub row: u8,
    pub column: u8,
    pub x: f32,
    pub y: f32,
}

const LIFETIME: f32 = 2.5;
const MAX_RIPPLES: usize = 128;

struct Ripple {
    coord: MatrixCoord,
    strength: f32,
    age: f32,
}

pub struct RippleSimulation {
    waves: Vec<Ripple>,
    previous: [f32; 256],
    cooldown: [f32; 256],
}

impl Default for RippleSimulation {
    fn default() -> Self {
        Self {
            waves: Vec::new(),
            previous: [0.0; 256],
            cooldown: [0.0; 256],
        }
    }
}

impl RippleSimulation {
    pub fn advance(&mut self, dt: f32, keys: impl IntoIterator<Item = (u16, f32)>) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        for wave in &mut self.waves {
            wave.age += dt;
        }
        self.waves.retain(|wave| wave.age < LIFETIME);
        for cooldown in &mut self.cooldown {
            *cooldown = (*cooldown - dt).max(0.0);
        }
        let mut current = [0.0f32; 256];
        for (key_code, pressure) in keys {
            // Do not truncate namespaces: custom/Fn keys are not ordinary USB HID keys.
            if key_code < 256 && pressure.is_finite() {
                let index = usize::from(key_code);
                current[index] = current[index].max(pressure.clamp(0.0, 1.0));
            }
        }
        for (index, &pressure) in current.iter().enumerate() {
            if pressure < 0.05 {
                continue;
            }
            let Some(coord) = hid_coord(index as u16) else {
                continue;
            };
            // A new touch is immediate. Held keys emit at most five rings/second;
            // their strength tracks key travel, rather than typing repeat events.
            if self.previous[index] < 0.05 || self.cooldown[index] == 0.0 {
                if self.waves.len() == MAX_RIPPLES {
                    self.waves.remove(0);
                }
                self.waves.push(Ripple {
                    coord,
                    strength: pressure,
                    age: 0.0,
                });
                self.cooldown[index] = 0.2;
            }
        }
        self.previous = current;
    }

    /// Number of active rings; bounded independently of the input rate.
    pub fn wave_count(&self) -> usize {
        self.waves.len()
    }

    /// Render RGB values in geometry order. The gradient callback preserves the
    /// caller's palette arithmetic without depending on its UI or SDK types.
    pub fn render(
        &self,
        geometry: &[KeyGeometry],
        gradient: impl Fn(u8) -> [u8; 3],
        brightness: u8,
        base_color: Option<[u8; 3]>,
        ripple_color: Option<[u8; 3]>,
    ) -> Vec<[u8; 3]> {
        let mut intensities = vec![0.0f32; geometry.len()];
        for wave in &self.waves {
            let Some(origin) = geometry
                .iter()
                .find(|key| key.row == wave.coord.row && key.column == wave.coord.column)
            else {
                continue;
            };
            let amplitude = wave.strength * (1.0 - wave.age / LIFETIME).powi(2);
            for (key, intensity) in geometry.iter().zip(&mut intensities) {
                let distance = (key.x - origin.x).hypot(key.y - origin.y);
                let ring = (1.0 - (distance - wave.age * 6.0).abs() / 1.1).max(0.0);
                *intensity += ring * amplitude;
            }
        }
        intensities
            .into_iter()
            .map(|intensity| {
                let intensity = intensity.min(1.0);
                let gradient = gradient((intensity * 255.0) as u8);
                if base_color.is_none() && ripple_color.is_none() {
                    // Keep legacy quantization and palette output byte-for-byte.
                    scale(gradient, (intensity * f32::from(brightness)) as u8)
                } else {
                    let base = base_color.unwrap_or([0; 3]);
                    let ripple = ripple_color.unwrap_or(gradient);
                    let channel = |i: usize| {
                        (f32::from(base[i]) * (1.0 - intensity) + f32::from(ripple[i]) * intensity)
                            as u8
                    };
                    scale([channel(0), channel(1), channel(2)], brightness)
                }
            })
            .collect()
    }
}

fn scale(color: [u8; 3], brightness: u8) -> [u8; 3] {
    color.map(|channel| ((u16::from(channel) * u16::from(brightness)) / 255) as u8)
}

/// Standard ANSI HID positions in the RGB SDK's 6x21 matrix (not OS text layout).
/// Restrict the first toy to the typing block; Fn/custom namespaces and navigation
/// are intentionally unmapped until validated on hardware. See the SDK's
/// resources/keyboard-matrix-rows-columns.png. Gaps are not compressed.
pub fn hid_coord(code: u16) -> Option<MatrixCoord> {
    const ROWS: [&[u16]; 4] = [
        &[
            0x35, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2d, 0x2e, 0x2a,
        ],
        &[
            0x2b, 0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13, 0x2f, 0x30, 0x31,
        ],
        &[
            0x39, 0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f, 0x33, 0x34, 0, 0x28,
        ],
        &[
            0xe1, 0, 0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0, 0xe5,
        ],
    ];
    if code == 0 {
        return None;
    }
    if code == 0x2c {
        return Some(MatrixCoord { row: 5, column: 6 });
    }
    ROWS.iter().enumerate().find_map(|(row, keys)| {
        keys.iter()
            .position(|&key| key == code)
            .map(|column| MatrixCoord {
                row: row as u8 + 1,
                column: column as u8,
            })
    })
}

/// Canonical engine geometry for the 80HE, including matrix gaps.
pub fn wooting_80he_geometry() -> Vec<KeyGeometry> {
    let row_offsets = [0.0, 0.25, 0.45, 0.7, 1.05, 0.0];
    (0..6u8)
        .flat_map(|row| {
            (0..17u8).map(move |column| KeyGeometry {
                row,
                column,
                x: f32::from(column) + row_offsets[usize::from(row)],
                y: f32::from(row),
            })
        })
        .collect()
}

/// Runtime palettes. Unknown names use the default Wooting palette.
pub fn palette_colors(name: &str) -> &'static [[u8; 3]] {
    match name {
        "cyberpunk" => &[[255, 0, 120], [0, 255, 255], [255, 220, 0]],
        "ocean" => &[[0, 32, 96], [0, 160, 220], [120, 255, 255]],
        "heat" => &[[80, 0, 0], [255, 64, 0], [255, 220, 64]],
        "terminal" => &[[0, 32, 0], [0, 220, 64], [180, 255, 180]],
        _ => &[[0, 180, 255], [255, 255, 255], [0, 90, 220]],
    }
}

pub fn palette_gradient(name: &str, position: u8) -> [u8; 3] {
    color_gradient(palette_colors(name), position)
}

/// The engine's integer gradient, including endpoint and truncation behavior.
/// An empty palette renders black.
pub fn color_gradient(colors: &[[u8; 3]], position: u8) -> [u8; 3] {
    if colors.is_empty() {
        return [0; 3];
    }
    if colors.len() == 1 {
        return colors[0];
    }
    if position == 255 {
        return colors[colors.len() - 1];
    }
    let segments = colors.len() - 1;
    let scaled = usize::from(position) * segments;
    let index = (scaled / 255).min(segments - 1);
    let amount = (scaled % 255) as u8;
    std::array::from_fn(|i| {
        let a = u16::from(colors[index][i]) * u16::from(255 - amount);
        let b = u16::from(colors[index + 1][i]) * u16::from(amount);
        ((a + b) / 255) as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_wave_has_exact_peak_decay_and_idle_colors() {
        let geometry = [
            KeyGeometry {
                row: 3,
                column: 4,
                x: 0.0,
                y: 0.0,
            },
            KeyGeometry {
                row: 3,
                column: 5,
                x: 1.2,
                y: 0.0,
            },
        ];
        let mut sim = RippleSimulation::default();
        let draw = |sim: &RippleSimulation, brightness| {
            sim.render(
                &geometry,
                |_| [255; 3],
                brightness,
                Some([10, 20, 30]),
                Some([210, 220, 230]),
            )
        };
        assert_eq!(draw(&sim, 255), vec![[10, 20, 30]; 2]);
        sim.advance(0.0, [(0x09, 1.0)]);
        assert_eq!(draw(&sim, 255), vec![[210, 220, 230], [10, 20, 30]]);
        sim.advance(0.2, []);
        // At radius 1.2, the ring has amplitude (1 - .2 / 2.5)^2 = .8464.
        assert_eq!(draw(&sim, 255), vec![[10, 20, 30], [179, 189, 199]]);
        assert_eq!(draw(&sim, 0), vec![[0; 3]; 2]);
        sim.advance(2.3, []);
        assert_eq!(draw(&sim, 255), vec![[10, 20, 30]; 2]);
    }

    #[test]
    fn legacy_quantization_and_palette_interpolation_are_exact() {
        assert_eq!(palette_gradient("ocean", 0), [0, 32, 96]);
        assert_eq!(palette_gradient("ocean", 127), [0, 159, 219]);
        assert_eq!(palette_gradient("ocean", 128), [0, 160, 220]);
        assert_eq!(palette_gradient("ocean", 255), [120, 255, 255]);
        let mut sim = RippleSimulation::default();
        sim.advance(0.0, [(0x09, 0.5)]);
        let geometry = [KeyGeometry {
            row: 3,
            column: 4,
            x: 0.0,
            y: 0.0,
        }];
        assert_eq!(
            sim.render(&geometry, |p| palette_gradient("ocean", p), 96, None, None),
            vec![[0, 29, 41]]
        );
        assert_eq!(
            sim.render(&[], |_| [255; 3], 255, None, None),
            Vec::<[u8; 3]>::new()
        );
    }

    #[test]
    fn input_namespaces_nonfinite_values_and_wave_count_are_bounded() {
        let mut sim = RippleSimulation::default();
        sim.advance(f32::NAN, [(0x0209, 1.0), (0x09, f32::NAN), (0, 1.0)]);
        assert_eq!(sim.wave_count(), 0);
        for _ in 0..10 {
            sim.advance(0.01, [(0x09, 1.0)]);
        }
        assert_eq!(sim.wave_count(), 1);
        sim.advance(0.2, [(0x09, 1.0)]);
        assert_eq!(sim.wave_count(), 2);
        for _ in 0..100 {
            sim.advance(0.2, (1..256).map(|code| (code, 1.0)));
        }
        assert_eq!(sim.wave_count(), MAX_RIPPLES);
        sim.advance(3.0, []);
        assert_eq!(sim.wave_count(), 0);
    }
}

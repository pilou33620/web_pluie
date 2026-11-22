//! Les images radar : décodage des tuiles RainViewer en réflectivité (dBZ), mosaïque de 3×3 tuiles
//! autour d'un point (projection Mercator du zoom 7), et rendu d'une grille en image PNG.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::LazyLock;

pub const ZOOM: u32 = 7; // le plus fin que RainViewer sert
pub const TUILE: usize = 256;
pub const COTE: usize = 3; // tuiles par côté de la mosaïque
pub const N: usize = TUILE * COTE; // 768 pixels, ~660 km sous nos latitudes
/// Réflectivité d'un pixel sans écho.
pub const SANS: f32 = -32.0;
/// Seuil de pluie : 12 dBZ ≈ 0,2 mm/h (Marshall-Palmer) ; en dessous, bruit et bruine qui ne mouille pas.
pub const SEUIL_DBZ: f32 = 12.0;

/// Palette « Universal Blue » de RainViewer (seule offerte depuis 2026), dBZ -10 à 75 : pluie.
/// Généré depuis https://www.rainviewer.com/files/rainviewer_api_colors_table.csv (RGBA).
const PALETTE_PLUIE: [u32; 86] = [
    0x63615914, 0x66635a19, 0x69665c1e, 0x6c685d24, 0x6f6b5f29, 0x726e612e, 0x75706234, 0x78736439,
    0x7c75653e, 0x7f786744, 0x827b6949, 0x857d6a4e, 0x88806c54, 0x8b826d59, 0x8e856f5e, 0x92887164,
    0x9e93756e, 0xaa9e7978, 0xb6a97e82, 0xc2b4828c, 0xcec08796, 0xd2c48ba0, 0xd6c88faa, 0xdacc93b4,
    0xded097be, 0x88ddeeff, 0x6cd1ebff, 0x51c5e8ff, 0x36bae5ff, 0x1baee2ff, 0x00a3e0ff, 0x009ad5ff,
    0x0091caff, 0x0088bfff, 0x007fb4ff, 0x0077aaff, 0x0070a3ff, 0x00699cff, 0x006295ff, 0x005b8eff,
    0x005588ff, 0x005180ff, 0x004e78ff, 0x004a70ff, 0x004768ff, 0xffee00ff, 0xffe000ff, 0xffd200ff,
    0xffc500ff, 0xffb700ff, 0xffaa00ff, 0xff9f00ff, 0xff9500ff, 0xff8b00ff, 0xff8100ff, 0xff4400ff,
    0xf23600ff, 0xe62800ff, 0xd91b00ff, 0xcd0d00ff, 0xc10000ff, 0xa80000ff, 0x8f0000ff, 0x760000ff,
    0x5d0000ff, 0xffaaffff, 0xff9fffff, 0xff95ffff, 0xff8bffff, 0xff81ffff, 0xff77ffff, 0xff6cffff,
    0xff62ffff, 0xff58ffff, 0xff4effff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
    0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0x00ff00ff,
];
/// La même pour la neige (tuiles demandées avec la neige en couleurs séparées).
const PALETTE_NEIGE: [u32; 86] = [
    0xcfffff00, 0xceffff0c, 0xcdffff19, 0xccffff26, 0xcbffff33, 0xcbffff3f, 0xcaffff4c, 0xc9ffff59,
    0xc8ffff66, 0xc7ffff72, 0xc7ffff7f, 0xc6ffff8c, 0xc5ffff99, 0xc4ffffa5, 0xc3ffffb2, 0xc3ffffbf,
    0xc2ffffcc, 0xc1ffffd8, 0xc0ffffe5, 0xbffffff2, 0xbfffffff, 0xb8f8ffff, 0xb2f2ffff, 0xabebffff,
    0xa5e5ffff, 0x9fdfffff, 0x98d8ffff, 0x92d2ffff, 0x8bcbffff, 0x85c5ffff, 0x7fbfffff, 0x78b8ffff,
    0x72b2ffff, 0x6babffff, 0x65a5ffff, 0x5f9fffff, 0x5b9bffff, 0x5898ffff, 0x5595ffff, 0x5292ffff,
    0x4f8fffff, 0x4b8bffff, 0x4888ffff, 0x4585ffff, 0x4282ffff, 0x3f7fffff, 0x3b7bffff, 0x3878ffff,
    0x3575ffff, 0x3272ffff, 0x2f6fffff, 0x2b6bffff, 0x2868ffff, 0x2565ffff, 0x2262ffff, 0x1f5fffff,
    0x1b5bffff, 0x1858ffff, 0x1555ffff, 0x1252ffff, 0x0f4fffff, 0x0c4bffff, 0x0948ffff, 0x0645ffff,
    0x0242ffff, 0x003fffff, 0x003bffff, 0x0038ffff, 0x0035ffff, 0x0032ffff, 0x002fffff, 0x002bffff,
    0x0028ffff, 0x0025ffff, 0x0022ffff, 0x001fffff, 0x001bffff, 0x0018ffff, 0x0015ffff, 0x0012ffff,
    0x000fffff, 0x000cffff, 0x0009ffff, 0x0006ffff, 0x0002ffff, 0x0000ffff,
];

/// Couleur RGBA → (dBZ, neige). Les doublons (blanc de 65 à 74 dBZ) gardent le plus faible.
static DECODAGE: LazyLock<HashMap<u32, (f32, bool)>> = LazyLock::new(|| {
    let mut t = HashMap::new();
    for (palette, neige) in [(&PALETTE_PLUIE, false), (&PALETTE_NEIGE, true)] {
        for (i, &c) in palette.iter().enumerate() {
            if c & 0xff != 0 {
                t.entry(c).or_insert((i as f32 - 10.0, neige));
            }
        }
    }
    t
});

/// Couleur hors palette (tuile recompressée) : la plus proche ; transparente : pas d'écho.
fn plus_proche(c: u32) -> (f32, bool) {
    if c & 0xff == 0 {
        return (SANS, false);
    }
    let ecart = |a: u32| (0..4).map(|k| (((a >> (8 * k)) & 0xff) as i32 - ((c >> (8 * k)) & 0xff) as i32).pow(2)).sum::<i32>();
    DECODAGE.iter().min_by_key(|(k, _)| ecart(**k)).map(|(_, v)| *v).unwrap_or((SANS, false))
}

/// Une grille de réflectivité, ligne par ligne, du nord-ouest au sud-est.
#[derive(Clone, Debug)]
pub struct Grille {
    pub l: usize,
    pub h: usize,
    pub dbz: Vec<f32>,
    pub neige: Vec<bool>,
}

impl Grille {
    pub fn vide(l: usize, h: usize) -> Grille {
        Grille { l, h, dbz: vec![SANS; l * h], neige: vec![false; l * h] }
    }

    pub fn get(&self, x: isize, y: isize) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.l || y as usize >= self.h {
            return SANS;
        }
        self.dbz[y as usize * self.l + x as usize]
    }

    /// La valeur du pixel qui contient le point (x, y) ; hors grille : pas d'écho.
    pub fn echantillon(&self, x: f32, y: f32) -> f32 {
        if !(x.is_finite() && y.is_finite()) {
            return SANS;
        }
        self.get(x.floor() as isize, y.floor() as isize)
    }

    /// Part des pixels de pluie dans un disque de `rayon` pixels autour de (x, y).
    pub fn part_pluie(&self, x: f32, y: f32, rayon: f32) -> f32 {
        let r = rayon.ceil() as isize;
        let (cx, cy) = (x.floor() as isize, y.floor() as isize);
        let (mut n, mut mouille) = (0u32, 0u32);
        for dy in -r..=r {
            for dx in -r..=r {
                if ((dx * dx + dy * dy) as f32) <= rayon * rayon {
                    n += 1;
                    mouille += (self.get(cx + dx, cy + dy) >= SEUIL_DBZ) as u32;
                }
            }
        }
        mouille as f32 / n.max(1) as f32
    }
}

/// Une tuile PNG de RainViewer (palette Universal Blue, sans lissage) → grille de dBZ.
pub fn decoder_tuile(octets: &[u8]) -> Result<Grille, String> {
    // borne la mémoire du décodage : une tuile de 256 × 256 RGBA fait 256 Ko
    let mut dec = png::Decoder::new_with_limits(octets, png::Limits { bytes: 4 << 20 });
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut lecteur = dec.read_info().map_err(|e| format!("tuile illisible : {e}"))?;
    let mut buf = vec![0; lecteur.output_buffer_size()];
    let info = lecteur.next_frame(&mut buf).map_err(|e| format!("tuile illisible : {e}"))?;
    let (l, h) = (info.width as usize, info.height as usize);
    let canaux = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err("tuile : palette non développée".into()),
    };
    if info.line_size != l * canaux || buf.len() < l * h * canaux {
        return Err("tuile : format inattendu".into());
    }
    let mut g = Grille::vide(l, h);
    let mut vues: HashMap<u32, (f32, bool)> = HashMap::new();
    for i in 0..l * h {
        let p = &buf[i * canaux..(i + 1) * canaux];
        let c = match canaux {
            4 => u32::from_be_bytes([p[0], p[1], p[2], p[3]]),
            3 => u32::from_be_bytes([p[0], p[1], p[2], 255]),
            2 => u32::from_be_bytes([p[0], p[0], p[0], p[1]]),
            _ => u32::from_be_bytes([p[0], p[0], p[0], 255]),
        };
        let (d, n) = *vues.entry(c).or_insert_with(|| DECODAGE.get(&c).copied().unwrap_or_else(|| plus_proche(c)));
        g.dbz[i] = d;
        g.neige[i] = n;
    }
    Ok(g)
}

/// Intensité de pluie (mm/h) d'une réflectivité, par Marshall-Palmer (Z = 200 R^1,6).
pub fn mm_h(dbz: f32) -> f32 {
    if dbz < SEUIL_DBZ {
        return 0.0;
    }
    (10f32.powf(dbz / 10.0) / 200.0).powf(1.0 / 1.6)
}

// --- Projection ---------------------------------------------------------------------

fn monde() -> f64 {
    (TUILE << ZOOM) as f64
}

/// (lat, lon) → pixel du monde au zoom 7 (Web Mercator, comme les tuiles).
pub fn monde_px(lat: f64, lon: f64) -> (f64, f64) {
    let phi = lat.clamp(-85.05, 85.05).to_radians();
    ((lon + 180.0) / 360.0 * monde(), (1.0 - (phi.tan() + 1.0 / phi.cos()).ln() / PI) / 2.0 * monde())
}

pub fn lat_lon(x: f64, y: f64) -> (f64, f64) {
    let lat = (PI * (1.0 - 2.0 * y / monde())).sinh().atan().to_degrees();
    (lat, x / monde() * 360.0 - 180.0)
}

/// Taille au sol d'un pixel (km) à cette latitude.
pub fn km_par_px(lat: f64) -> f64 {
    40075.016686 * lat.to_radians().cos() / monde()
}

/// La mosaïque de 3×3 tuiles centrée sur la tuile (tx, ty).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Domaine {
    pub tx: u32,
    pub ty: u32,
}

impl Domaine {
    pub fn autour(lat: f64, lon: f64) -> Domaine {
        let (x, y) = monde_px(lat, lon);
        let max = (1u32 << ZOOM) - 2; // la mosaïque reste dans la carte
        let t = |v: f64| ((v / TUILE as f64).floor().max(0.0) as u32).clamp(1, max);
        Domaine { tx: t(x), ty: t(y) }
    }

    fn origine(&self) -> (f64, f64) {
        (((self.tx - 1) as usize * TUILE) as f64, ((self.ty - 1) as usize * TUILE) as f64)
    }

    /// Position du point dans la mosaïque (pixels).
    pub fn px(&self, lat: f64, lon: f64) -> (f32, f32) {
        let ((x, y), (ox, oy)) = (monde_px(lat, lon), self.origine());
        ((x - ox) as f32, (y - oy) as f32)
    }

    pub fn lat_lon(&self, x: f32, y: f32) -> (f64, f64) {
        let (ox, oy) = self.origine();
        lat_lon(ox + x as f64, oy + y as f64)
    }

    /// Les 9 tuiles : (colonne, ligne, x, y) ; ligne par ligne.
    pub fn tuiles(&self) -> Vec<(usize, usize, u32, u32)> {
        (0..COTE * COTE).map(|k| (k % COTE, k / COTE, self.tx - 1 + (k % COTE) as u32, self.ty - 1 + (k / COTE) as u32)).collect()
    }

    /// [nord, ouest, sud, est] de la mosaïque.
    pub fn bornes(&self) -> [f64; 4] {
        let (n, o) = self.lat_lon(0.0, 0.0);
        let (s, e) = self.lat_lon(N as f32, N as f32);
        [n, o, s, e]
    }
}

/// Les 9 tuiles décodées (ordre de `Domaine::tuiles`) → la mosaïque.
pub fn assembler(tuiles: &[Grille]) -> Result<Grille, String> {
    if tuiles.len() != COTE * COTE || tuiles.iter().any(|t| t.l != TUILE || t.h != TUILE) {
        return Err("tuiles radar incomplètes".into());
    }
    let mut g = Grille::vide(N, N);
    for (k, t) in tuiles.iter().enumerate() {
        let (c, l) = (k % COTE, k / COTE);
        for y in 0..TUILE {
            let dest = (l * TUILE + y) * N + c * TUILE;
            g.dbz[dest..dest + TUILE].copy_from_slice(&t.dbz[y * TUILE..(y + 1) * TUILE]);
            g.neige[dest..dest + TUILE].copy_from_slice(&t.neige[y * TUILE..(y + 1) * TUILE]);
        }
    }
    Ok(g)
}

/// La grille en PNG, aux couleurs des tuiles RainViewer (transparent sans écho).
pub fn image(g: &Grille) -> Result<Vec<u8>, String> {
    let mut pixels = Vec::with_capacity(g.l * g.h * 4);
    for (d, n) in g.dbz.iter().zip(&g.neige) {
        let c = if *d < -10.0 {
            0
        } else {
            let i = ((d.round() + 10.0) as usize).min(85);
            if *n { PALETTE_NEIGE[i] } else { PALETTE_PLUIE[i] }
        };
        pixels.extend_from_slice(&c.to_be_bytes());
    }
    let mut sortie = Vec::new();
    let mut enc = png::Encoder::new(&mut sortie, g.l as u32, g.h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut ecr = enc.write_header().map_err(|e| e.to_string())?;
    ecr.write_image_data(&pixels).map_err(|e| e.to_string())?;
    ecr.finish().map_err(|e| e.to_string())?;
    Ok(sortie)
}

#[cfg(test)]
mod essais {
    use super::*;

    #[test]
    fn tuile_reelle_decodee() {
        // tuile 7/63/45 du 22/11/2026 vers 9 h (pluie sur la Gironde) ; chiffres relevés par un script à part
        let g = decoder_tuile(include_bytes!("../essais/tuile_rainviewer_7_63_45.png")).unwrap();
        assert_eq!((g.l, g.h), (256, 256));
        assert_eq!(g.dbz.iter().filter(|d| **d > SANS).count(), 17832);
        assert_eq!(g.dbz.iter().filter(|d| **d >= SEUIL_DBZ).count(), 12136);
        assert_eq!(g.dbz.iter().filter(|d| **d >= 35.0).count(), 1171);
        assert_eq!(g.dbz.iter().cloned().fold(SANS, f32::max), 46.0);
        assert!(!g.neige.iter().any(|n| *n));
        assert!(decoder_tuile(b"pas une image").is_err());
    }

    #[test]
    fn image_relue_a_l_identique() {
        let mut g = Grille::vide(16, 8);
        for (i, d) in g.dbz.iter_mut().enumerate() {
            *d = (i as f32 * 0.7).round() - 10.0;
        }
        g.neige[5] = true;
        let relue = decoder_tuile(&image(&g).unwrap()).unwrap();
        for i in 0..g.dbz.len() {
            // 65 à 74 dBZ partagent le blanc ; au-delà de 75, le vert
            let attendu = if (65.0..75.0).contains(&g.dbz[i]) { 65.0 } else { g.dbz[i].min(75.0) };
            assert_eq!((relue.dbz[i], relue.neige[i]), (attendu, g.neige[i]), "pixel {i}");
        }
        assert_eq!(plus_proche(0x0091cbff).0, 22.0, "couleur voisine de 22 dBZ");
        assert_eq!(plus_proche(0x12345600).0, SANS);
    }

    #[test]
    fn projection_et_domaine() {
        let (lat, lon) = (44.8065, -0.6303); // Pessac
        let (x, y) = monde_px(lat, lon);
        let (la, lo) = lat_lon(x, y);
        assert!((la - lat).abs() < 1e-9 && (lo - lon).abs() < 1e-9);
        let d = Domaine::autour(lat, lon);
        assert_eq!((d.tx, d.ty), (63, 46));
        let (px, py) = d.px(lat, lon);
        assert!((256.0..512.0).contains(&px) && (256.0..512.0).contains(&py), "le point est dans la tuile du centre");
        let (la, lo) = d.lat_lon(px, py);
        assert!((la - lat).abs() < 1e-6 && (lo - lon).abs() < 1e-6);
        assert_eq!(d.tuiles()[0], (0, 0, 62, 45));
        assert_eq!(d.tuiles()[8], (2, 2, 64, 47));
        let [n, o, s, e] = d.bornes();
        assert!(n > lat && s < lat && o < lon && e > lon);
        assert!((km_par_px(lat) - 0.8677).abs() < 0.001);
        assert_eq!(Domaine::autour(85.0, -180.0), Domaine { tx: 1, ty: 1 }, "bord de la carte");
    }

    #[test]
    fn intensites_et_mosaique() {
        assert_eq!(mm_h(5.0), 0.0);
        assert!((mm_h(12.0) - 0.205).abs() < 0.01);
        assert!((mm_h(46.0) - 27.3).abs() < 0.5);
        let mut tuiles = vec![Grille::vide(TUILE, TUILE); 9];
        tuiles[4].dbz[0] = 30.0;
        tuiles[8].dbz[TUILE * TUILE - 1] = 40.0;
        let g = assembler(&tuiles).unwrap();
        assert_eq!(g.get(256, 256), 30.0);
        assert_eq!(g.get(767, 767), 40.0);
        assert_eq!(g.get(768, 0), SANS);
        assert!(assembler(&tuiles[..8]).is_err());
        let mut p = Grille::vide(20, 20);
        p.dbz[10 * 20 + 10] = 20.0;
        assert!((p.part_pluie(10.5, 10.5, 0.5) - 1.0).abs() < 1e-6);
        assert!(p.part_pluie(10.5, 10.5, 2.0) < 0.1);
    }
}

//! Le mouvement de la pluie. Flux optique par appariement de blocs : chaque bloc d'une image est
//! cherché dans la suivante (corrélation normalisée, cumulée sur plusieurs paires d'images, affinée
//! au sous-pixel), puis les vecteurs sont lissés en un champ, le vecteur global comblant les zones
//! sèches. L'advection semi-lagrangienne prolonge ensuite ce mouvement : pour savoir ce qui sera en
//! un point dans t minutes, on remonte la trajectoire de t minutes et on lit l'image actuelle.

use crate::radar::{Grille, N, SANS};

const F: usize = 4; // réduction pour le calcul : 4 pixels → ~3,5 km
const NC: usize = N / F; // 192
const DEMI_BLOC: isize = 8; // blocs de 16 × 16 pixels réduits (~55 km)
const PAS: usize = 8; // un bloc tous les 8 pixels réduits : 24 × 24 nœuds
const RAYON: isize = 8; // recherche à ±8 pixels réduits par image (~165 km/h pour 10 min)
const QUALITE_MIN: f32 = 0.5; // corrélation en dessous de laquelle un bloc ne dit rien
const PLUIE_MIN: usize = 26; // pixels de pluie (sur 256) pour qu'un bloc compte
const SIGMA_NOEUDS: f32 = 1.5; // lissage du champ (~40 km)
const POIDS_GLOBAL: f32 = 0.1;

/// Champ de vecteurs aux nœuds d'une grille régulière, en pixels (pleine résolution) par minute ;
/// x vers l'est, y vers le sud.
#[derive(Clone, Debug)]
pub struct Champ {
    pub n: usize,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
    /// Mouvement d'ensemble de la pluie du domaine.
    pub global: (f32, f32),
    /// Corrélation moyenne des blocs retenus (0 à 1), et leur nombre.
    pub qualite: f32,
    pub blocs: usize,
}

/// Position d'un nœud (pixels pleine résolution).
pub fn noeud(i: usize) -> f32 {
    ((PAS / 2 + i * PAS) as f32 + 0.5) * F as f32
}

impl Champ {
    pub fn uniforme(u: f32, v: f32) -> Champ {
        let n = NC / PAS;
        Champ { n, u: vec![u; n * n], v: vec![v; n * n], global: (u, v), qualite: 1.0, blocs: 0 }
    }

    /// Vitesse au point (x, y), interpolée entre les nœuds (bord : le nœud le plus proche).
    pub fn vitesse(&self, x: f32, y: f32) -> (f32, f32) {
        let pas = (PAS * F) as f32;
        let gx = ((x - noeud(0)) / pas).clamp(0.0, (self.n - 1) as f32);
        let gy = ((y - noeud(0)) / pas).clamp(0.0, (self.n - 1) as f32);
        if !(gx.is_finite() && gy.is_finite()) {
            return self.global;
        }
        let (i, j) = ((gx as usize).min(self.n - 2), (gy as usize).min(self.n - 2));
        let (fx, fy) = (gx - i as f32, gy - j as f32);
        let k = |a: usize, b: usize| b * self.n + a;
        let mix = |t: &[f32]| {
            let haut = t[k(i, j)] * (1.0 - fx) + t[k(i + 1, j)] * fx;
            let bas = t[k(i, j + 1)] * (1.0 - fx) + t[k(i + 1, j + 1)] * fx;
            haut * (1.0 - fy) + bas * fy
        };
        (mix(&self.u), mix(&self.v))
    }

    /// Un pas de trajectoire en arrière (point milieu) : où était, `dt` minutes plus tôt, ce qui
    /// est en (x, y). `dv` s'ajoute au champ (perturbation d'un membre de l'ensemble).
    pub fn reculer(&self, (x, y): (f32, f32), dt: f32, dv: (f32, f32)) -> (f32, f32) {
        let (u1, v1) = self.vitesse(x, y);
        let (xm, ym) = (x - (u1 + dv.0) * dt / 2.0, y - (v1 + dv.1) * dt / 2.0);
        let (u2, v2) = self.vitesse(xm, ym);
        (x - (u2 + dv.0) * dt, y - (v2 + dv.1) * dt)
    }
}

/// Image réduite pour la corrélation : moyenne des dBZ au-dessus de 10 (le bruit faible ne compte pas).
fn reduire(g: &Grille) -> Vec<f32> {
    let mut r = vec![0.0; NC * NC];
    for y in 0..NC {
        for x in 0..NC {
            let mut s = 0.0;
            for dy in 0..F {
                for dx in 0..F {
                    s += (g.dbz[(y * F + dy) * g.l + x * F + dx] - 10.0).clamp(0.0, 45.0);
                }
            }
            r[y * NC + x] = s / (F * F) as f32;
        }
    }
    r
}

/// Corrélation normalisée du bloc (centre c, demi-côté d) de `a` avec `b` décalé, pour chaque
/// décalage de ±RAYON ; ajoutée à `cumul`. Faux si le bloc n'a pas assez de pluie.
fn correler(a: &[f32], b: &[f32], (cx, cy): (isize, isize), d: isize, pluie_min: usize, cumul: &mut [f32]) -> bool {
    let lire = |t: &[f32], x: isize, y: isize| if x < 0 || y < 0 || x >= NC as isize || y >= NC as isize { 0.0 } else { t[y as usize * NC + x as usize] };
    let (mut saa, mut mouilles) = (0.0f32, 0usize);
    for y in cy - d..cy + d {
        for x in cx - d..cx + d {
            let v = lire(a, x, y);
            saa += v * v;
            mouilles += (v > 0.0) as usize;
        }
    }
    if mouilles < pluie_min || saa <= 0.0 {
        return false;
    }
    let cote = (2 * RAYON + 1) as usize;
    for oy in -RAYON..=RAYON {
        for ox in -RAYON..=RAYON {
            let (mut sab, mut sbb) = (0.0f32, 0.0f32);
            for y in cy - d..cy + d {
                for x in cx - d..cx + d {
                    let va = lire(a, x, y);
                    let vb = lire(b, x + ox, y + oy);
                    sab += va * vb;
                    sbb += vb * vb;
                }
            }
            if sbb > 0.0 {
                cumul[(oy + RAYON) as usize * cote + (ox + RAYON) as usize] += sab / (saa * sbb).sqrt();
            }
        }
    }
    true
}

/// Le meilleur décalage d'une surface de corrélation, affiné par une parabole : (dx, dy, score).
fn sommet(s: &[f32]) -> (f32, f32, f32) {
    let cote = (2 * RAYON + 1) as usize;
    let (k, &max) = s.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap_or((0, &0.0));
    let (i, j) = (k % cote, k / cote);
    let affiner = |m: f32, c: f32, p: f32| {
        let den = m - 2.0 * c + p;
        if den < 0.0 { ((m - p) / (2.0 * den)).clamp(-0.5, 0.5) } else { 0.0 }
    };
    let fx = if i > 0 && i < cote - 1 { affiner(s[k - 1], max, s[k + 1]) } else { 0.0 };
    let fy = if j > 0 && j < cote - 1 { affiner(s[k - cote], max, s[k + cote]) } else { 0.0 };
    (i as f32 - RAYON as f32 + fx, j as f32 - RAYON as f32 + fy, max)
}

/// Le champ de mouvement d'images successives (de la plus ancienne à la plus récente, toutes les
/// `dt` minutes). Sans pluie nulle part : champ nul, qualité 0.
pub fn estimer(images: &[&Grille], dt: f32) -> Champ {
    assert!(images.iter().all(|g| g.l == N && g.h == N), "mosaïque attendue");
    let reduites: Vec<Vec<f32>> = images.iter().map(|g| reduire(g)).collect();
    let paires: Vec<(&[f32], &[f32])> = reduites.windows(2).map(|w| (&w[0][..], &w[1][..])).collect();
    let cote = (2 * RAYON + 1) as usize;
    let vers_vitesse = |d: f32| d * F as f32 / dt;

    // mouvement d'ensemble : tout le domaine comme un seul bloc
    let mut s = vec![0.0; cote * cote];
    let centre = (NC / 2) as isize;
    let comptes = paires.iter().filter(|(a, b)| correler(a, b, (centre, centre), centre - RAYON, PLUIE_MIN, &mut s)).count();
    let global = if comptes > 0 {
        let (dx, dy, _) = sommet(&s);
        (vers_vitesse(dx), vers_vitesse(dy))
    } else {
        (0.0, 0.0)
    };

    // un vecteur par bloc assez pluvieux et assez bien corrélé
    let n = NC / PAS;
    let mut trouves = Vec::new(); // (i, j, u, v, qualité)
    for j in 0..n {
        for i in 0..n {
            let c = ((PAS / 2 + i * PAS) as isize, (PAS / 2 + j * PAS) as isize);
            s.iter_mut().for_each(|x| *x = 0.0);
            let k = paires.iter().filter(|(a, b)| correler(a, b, c, DEMI_BLOC, PLUIE_MIN, &mut s)).count();
            if k == 0 {
                continue;
            }
            s.iter_mut().for_each(|x| *x /= k as f32);
            let (dx, dy, q) = sommet(&s);
            if q >= QUALITE_MIN {
                trouves.push((i, j, vers_vitesse(dx), vers_vitesse(dy), q));
            }
        }
    }

    // lissage gaussien pondéré par la qualité, le global comble les vides
    let mut champ = Champ::uniforme(global.0, global.1);
    champ.blocs = trouves.len();
    champ.qualite = if trouves.is_empty() { 0.0 } else { trouves.iter().map(|t| t.4).sum::<f32>() / trouves.len() as f32 };
    for j in 0..n {
        for i in 0..n {
            let (mut su, mut sv, mut sw) = (POIDS_GLOBAL * global.0, POIDS_GLOBAL * global.1, POIDS_GLOBAL);
            for &(a, b, u, v, q) in &trouves {
                let d2 = ((a as f32 - i as f32).powi(2) + (b as f32 - j as f32).powi(2)) / (SIGMA_NOEUDS * SIGMA_NOEUDS);
                if d2 < 16.0 {
                    let w = q * (-d2 / 2.0).exp();
                    (su, sv, sw) = (su + w * u, sv + w * v, sw + w);
                }
            }
            champ.u[j * n + i] = su / sw;
            champ.v[j * n + i] = sv / sw;
        }
    }
    champ
}

/// Prévision déterministe : l'image `g` transportée par le champ, aux échéances données (minutes,
/// croissantes), sur une grille réduite de `pas` (la valeur lue est celle du pixel d'origine).
pub fn transporter(g: &Grille, champ: &Champ, echeances: &[u32], pas: usize) -> Vec<Grille> {
    let (l, h) = (g.l / pas, g.h / pas);
    let mut pos: Vec<(f32, f32)> = (0..l * h).map(|k| (((k % l) as f32 + 0.5) * pas as f32, ((k / l) as f32 + 0.5) * pas as f32)).collect();
    let (mut t, dt) = (0u32, 5u32);
    let mut sorties = Vec::new();
    for &e in echeances {
        while t < e {
            let d = dt.min(e - t);
            pos.iter_mut().for_each(|p| *p = champ.reculer(*p, d as f32, (0.0, 0.0)));
            t += d;
        }
        let mut s = Grille::vide(l, h);
        for (k, &(x, y)) in pos.iter().enumerate() {
            let (xi, yi) = (x.floor() as isize, y.floor() as isize);
            s.dbz[k] = g.get(xi, yi);
            if s.dbz[k] > SANS {
                s.neige[k] = g.neige[yi as usize * g.l + xi as usize];
            }
        }
        sorties.push(s);
    }
    sorties
}

#[cfg(test)]
pub mod essais {
    use super::*;

    /// Un champ de pluie texturé (averses gaussiennes), décalé de (ox, oy) pixels.
    pub fn averses(ox: f32, oy: f32) -> Grille {
        let centres = [(300.0, 280.0, 45.0, 30.0), (420.0, 350.0, 30.0, 18.0), (360.0, 470.0, 40.0, 24.0), (250.0, 420.0, 35.0, 14.0),
            (480.0, 260.0, 38.0, 20.0), (520.0, 480.0, 42.0, 26.0), (200.0, 300.0, 33.0, 16.0), (380.0, 200.0, 36.0, 22.0)];
        let mut g = Grille::vide(N, N);
        for y in 0..N {
            for x in 0..N {
                let (px, py) = (x as f32 + 0.5 - ox, y as f32 + 0.5 - oy);
                let mut d = SANS;
                for &(cx, cy, max, r) in &centres {
                    let v = max - 30.0 * ((px - cx).powi(2) + (py - cy).powi(2)) / (r * r);
                    d = d.max(v);
                }
                g.dbz[y * N + x] = if d >= -10.0 { d.round() } else { SANS };
            }
        }
        g
    }

    #[test]
    fn mouvement_retrouve() {
        // pluie qui file à (1,2 ; -0,6) pixel par minute (~65 km/h vers l'est-nord-est)
        let (u, v) = (1.2, -0.6);
        let images: Vec<Grille> = (0..4).map(|k| averses(u * 10.0 * k as f32, v * 10.0 * k as f32)).collect();
        let refs: Vec<&Grille> = images.iter().collect();
        let c = estimer(&refs, 10.0);
        assert!((c.global.0 - u).abs() < 0.1 && (c.global.1 - v).abs() < 0.1, "global {:?}", c.global);
        assert!(c.blocs >= 8 && c.qualite > 0.8, "{} blocs, qualité {}", c.blocs, c.qualite);
        for (x, y) in [(330.0, 300.0), (400.0, 420.0), (100.0, 700.0)] {
            let (a, b) = c.vitesse(x, y);
            assert!((a - u).abs() < 0.15 && (b - v).abs() < 0.15, "({x}, {y}) : ({a}, {b})");
        }
        let sec = Grille::vide(N, N);
        let c = estimer(&[&sec, &sec], 10.0);
        assert_eq!((c.global, c.blocs, c.qualite), ((0.0, 0.0), 0, 0.0));
    }

    #[test]
    fn transport_et_trajectoire() {
        let proche = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        let c = Champ::uniforme(1.0, 0.5);
        assert!(proche(c.reculer((100.0, 100.0), 10.0, (0.0, 0.0)), (90.0, 95.0)));
        assert!(proche(c.reculer((100.0, 100.0), 10.0, (0.2, 0.0)), (88.0, 95.0)));
        // demi-pixels de déplacement : la lecture ne tombe jamais sur un bord de pixel
        let c = Champ::uniforme(1.025, 0.525);
        let g = averses(0.0, 0.0);
        let p = transporter(&g, &c, &[10, 20], 2);
        assert_eq!((p.len(), p[0].l), (2, N / 2));
        // le pixel réduit (k, m) a son centre en (2k+1, 2m+1) : à +20 min il lit (2k+1-20,5 ; 2m+1-10,5)
        for (k, m) in [(150isize, 140isize), (200, 230)] {
            assert_eq!(p[1].dbz[m as usize * p[1].l + k as usize], g.get(2 * k - 20, 2 * m - 10));
        }
        assert_eq!(p[1].dbz[0], SANS, "ce qui entre par le bord : rien");
    }
}

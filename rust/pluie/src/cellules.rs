//! Suivi de cellules : chaque averse forte (≥ 35 dBZ) est repérée comme un objet sur chaque image,
//! appariée d'une image à l'autre (position prévue la plus proche), et sa trajectoire prolongée.
//! Une cellule suivie sur au moins 3 images corrige localement le champ de mouvement : une averse
//! orageuse ne suit pas toujours le flot de la pluie qui l'entoure.

use crate::mouvement::{noeud, Champ};
use crate::radar::Grille;

pub const SEUIL_CELLULE: f32 = 35.0;
const R: usize = 2; // repérage sur une grille réduite de moitié (~1,7 km)
const AIRE_MIN: usize = 6; // pixels réduits (~18 km²)
const VITESSE_MAX: f32 = 3.0; // pixels par minute (~155 km/h) : au-delà, appariement douteux

/// Une averse sur une image : centre (pixels), aire (pixels), réflectivité maximale.
#[derive(Clone, Debug, PartialEq)]
pub struct Objet {
    pub x: f32,
    pub y: f32,
    pub aire: usize,
    pub dbz_max: f32,
}

impl Objet {
    pub fn rayon(&self) -> f32 {
        (self.aire as f32 / std::f32::consts::PI).sqrt()
    }
}

/// Les averses fortes d'une image (composantes connexes, 8 voisins).
pub fn reperer(g: &Grille) -> Vec<Objet> {
    let (l, h) = (g.l / R, g.h / R);
    let mut fort = vec![f32::NAN; l * h];
    for y in 0..h {
        for x in 0..l {
            let mut m = f32::MIN;
            for d in 0..R * R {
                m = m.max(g.dbz[(y * R + d / R) * g.l + x * R + d % R]);
            }
            if m >= SEUIL_CELLULE {
                fort[y * l + x] = m;
            }
        }
    }
    let mut vu = vec![false; l * h];
    let mut objets = Vec::new();
    for depart in 0..l * h {
        if vu[depart] || fort[depart].is_nan() {
            continue;
        }
        let (mut pile, mut aire) = (vec![depart], 0usize);
        let (mut sx, mut sy, mut sw, mut max) = (0.0f32, 0.0f32, 0.0f32, f32::MIN);
        vu[depart] = true;
        while let Some(k) = pile.pop() {
            let (x, y) = (k % l, k / l);
            let w = fort[k] - SEUIL_CELLULE + 1.0;
            (sx, sy, sw, max) = (sx + w * x as f32, sy + w * y as f32, sw + w, max.max(fort[k]));
            aire += 1;
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= l as isize || ny >= h as isize {
                    continue;
                }
                let v = ny as usize * l + nx as usize;
                if !vu[v] && !fort[v].is_nan() {
                    vu[v] = true;
                    pile.push(v);
                }
            }
        }
        if aire >= AIRE_MIN {
            let r = R as f32;
            objets.push(Objet { x: (sx / sw + 0.5) * r, y: (sy / sw + 0.5) * r, aire: aire * R * R, dbz_max: max });
        }
    }
    objets
}

/// Une cellule suivie jusqu'à la dernière image.
#[derive(Clone, Debug)]
pub struct Cellule {
    /// Centres successifs, une image après l'autre ; le dernier est sur l'image la plus récente.
    pub passe: Vec<(f32, f32)>,
    pub aire: usize,
    pub dbz_max: f32,
    /// Vitesse propre (pixels par minute), dès deux images.
    pub vitesse: Option<(f32, f32)>,
}

impl Cellule {
    pub fn rayon(&self) -> f32 {
        (self.aire as f32 / std::f32::consts::PI).sqrt()
    }

    /// Position prévue `t` minutes après la dernière image.
    pub fn position(&self, t: f32, champ: &Champ) -> (f32, f32) {
        let (x, y) = *self.passe.last().unwrap_or(&(0.0, 0.0));
        let (u, v) = self.vitesse.unwrap_or_else(|| champ.vitesse(x, y));
        (x + u * t, y + v * t)
    }

    /// Première minute (après la dernière image, jusqu'à `horizon`) où la cellule couvre le point,
    /// avec une marge qui grandit avec l'échéance.
    pub fn arrivee(&self, (px, py): (f32, f32), horizon: u32, champ: &Champ) -> Option<u32> {
        (0..=horizon).find(|&t| {
            let (x, y) = self.position(t as f32, champ);
            ((x - px).powi(2) + (y - py).powi(2)).sqrt() <= self.rayon() + 1.0 + 0.15 * t as f32
        })
    }
}

/// Pente des moindres carrés de positions prises toutes les `dt` minutes (4 dernières au plus).
fn pente(points: &[(f32, f32)], dt: f32) -> (f32, f32) {
    let p = &points[points.len().saturating_sub(4)..];
    let n = p.len() as f32;
    let tm = (n - 1.0) / 2.0;
    let (xm, ym) = (p.iter().map(|q| q.0).sum::<f32>() / n, p.iter().map(|q| q.1).sum::<f32>() / n);
    let (mut sx, mut sy, mut st) = (0.0, 0.0, 0.0);
    for (i, q) in p.iter().enumerate() {
        let t = i as f32 - tm;
        (sx, sy, st) = (sx + t * (q.0 - xm), sy + t * (q.1 - ym), st + t * t);
    }
    (sx / st / dt, sy / st / dt)
}

/// Suit les averses d'images successives (de la plus ancienne à la plus récente, toutes les `dt`
/// minutes) ; rend celles présentes sur la dernière image.
pub fn suivre(images: &[&Grille], dt: f32, champ: &Champ) -> Vec<Cellule> {
    let mut actives: Vec<Cellule> = Vec::new();
    for (k, g) in images.iter().enumerate() {
        let objets = reperer(g);
        if k == 0 {
            actives = objets.iter().map(|o| Cellule { passe: vec![(o.x, o.y)], aire: o.aire, dbz_max: o.dbz_max, vitesse: None }).collect();
            continue;
        }
        // appariement glouton, du couple le plus proche au plus lointain
        let mut couples = Vec::new();
        for (a, c) in actives.iter().enumerate() {
            let (px, py) = c.position(dt, champ);
            for (b, o) in objets.iter().enumerate() {
                let d = ((o.x - px).powi(2) + (o.y - py).powi(2)).sqrt();
                if d <= (2.0 * o.rayon()).max(15.0) {
                    couples.push((d, a, b));
                }
            }
        }
        couples.sort_by(|x, y| x.0.total_cmp(&y.0));
        let (mut pris_a, mut pris_b) = (vec![false; actives.len()], vec![false; objets.len()]);
        let mut suivantes = Vec::new();
        for (_, a, b) in couples {
            if pris_a[a] || pris_b[b] {
                continue;
            }
            (pris_a[a], pris_b[b]) = (true, true);
            let mut c = actives[a].clone();
            let o = &objets[b];
            c.passe.push((o.x, o.y));
            (c.aire, c.dbz_max) = (o.aire, o.dbz_max);
            let v = pente(&c.passe, dt);
            c.vitesse = (v.0.hypot(v.1) <= VITESSE_MAX).then_some(v);
            suivantes.push(c);
        }
        for (b, o) in objets.iter().enumerate() {
            if !pris_b[b] {
                suivantes.push(Cellule { passe: vec![(o.x, o.y)], aire: o.aire, dbz_max: o.dbz_max, vitesse: None });
            }
        }
        actives = suivantes;
    }
    actives
}

/// Le champ prend, autour de chaque cellule suivie sur 3 images ou plus, la vitesse de la cellule.
pub fn corriger(champ: &mut Champ, cellules: &[Cellule]) {
    for c in cellules.iter().filter(|c| c.passe.len() >= 3) {
        let (Some((cu, cv)), Some(&(x, y))) = (c.vitesse, c.passe.last()) else { continue };
        let portee = (2.0 * c.rayon()).max(24.0);
        for j in 0..champ.n {
            for i in 0..champ.n {
                let d2 = ((noeud(i) - x).powi(2) + (noeud(j) - y).powi(2)) / (portee * portee);
                let w = (-d2).exp();
                let k = j * champ.n + i;
                champ.u[k] += w * (cu - champ.u[k]);
                champ.v[k] += w * (cv - champ.v[k]);
            }
        }
    }
}

#[cfg(test)]
mod essais {
    use super::*;
    use crate::radar::{N, SANS};

    fn cellule_en(x0: f32, y0: f32) -> Grille {
        let mut g = Grille::vide(N, N);
        for y in 0..N {
            for x in 0..N {
                let d2 = (x as f32 + 0.5 - x0).powi(2) + (y as f32 + 0.5 - y0).powi(2);
                let v = 50.0 - d2 / 8.0; // ~13 pixels de rayon à 35 dBZ
                g.dbz[y * N + x] = if v > 10.0 { v.round() } else { SANS };
            }
        }
        g.dbz[10 * N + 700] = 40.0; // un pixel isolé : trop petit pour une cellule
        g
    }

    #[test]
    fn reperage() {
        let o = reperer(&cellule_en(300.0, 200.0));
        assert_eq!(o.len(), 1, "{o:?}");
        assert!((o[0].x - 300.0).abs() < 1.0 && (o[0].y - 200.0).abs() < 1.0, "{o:?}");
        assert_eq!(o[0].dbz_max, 50.0);
        assert!((o[0].rayon() - 11.0).abs() < 3.0, "rayon {}", o[0].rayon());
    }

    #[test]
    fn suivi_et_arrivee() {
        // la cellule file vers l'est à 1,5 pixel par minute, contre un champ général vers le nord
        let images: Vec<Grille> = (0..4).map(|k| cellule_en(200.0 + 15.0 * k as f32, 400.0)).collect();
        let refs: Vec<&Grille> = images.iter().collect();
        let mut champ = Champ::uniforme(0.0, -1.0);
        let c = suivre(&refs, 10.0, &champ);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].passe.len(), 4);
        let (u, v) = c[0].vitesse.unwrap();
        assert!((u - 1.5).abs() < 0.1 && v.abs() < 0.1, "({u}, {v})");
        // un point 60 pixels à l'est du dernier centre : couvert vers 60 / 1,5 = 40 min, marge déduite
        let t = c[0].arrivee((305.0, 400.0), 90, &champ).unwrap();
        assert!((25..=40).contains(&t), "arrivée {t}");
        assert_eq!(c[0].arrivee((245.0, 200.0), 90, &champ), None, "point hors de la trajectoire");
        corriger(&mut champ, &c);
        let (a, b) = champ.vitesse(noeud(7), noeud(12)); // le nœud le plus proche de la cellule
        assert!((a - 1.5).abs() < 0.3 && (b + 0.0).abs() < 0.3, "près de la cellule : ({a}, {b})");
        let (a, b) = champ.vitesse(700.0, 100.0);
        assert!(a.abs() < 0.01 && (b + 1.0).abs() < 0.01, "loin : ({a}, {b})");
    }
}

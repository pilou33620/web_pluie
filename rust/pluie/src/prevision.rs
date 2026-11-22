//! La probabilité de pluie sur un point. Ensemble de 40 membres : chacun remonte, minute par minute,
//! la trajectoire qui arrive sur le point, avec une vitesse perturbée, une croissance ou un déclin
//! tiré autour de la tendance mesurée, et un décalage qui grandit avec l'échéance. La probabilité
//! est la part des membres mouillés. Au-delà de 20 min, elle se mélange à celle d'AROME (part des
//! points voisins où le modèle fait pleuvoir), dont le poids grandit avec l'échéance.

use serde_json::{json, Value};

use crate::mouvement::Champ;
use crate::radar::{mm_h, Grille, SANS, SEUIL_DBZ};

pub const MEMBRES: usize = 40;
/// Minutes prévues après la dernière image (qui a souvent 10 à 20 min de retard sur l'heure).
pub const HORIZON: usize = 90;

// shortcut: incertitudes et poids fixés à la main ; à régler avec le bilan de fiabilité (pluie --bilan)
const ERREUR_VITESSE: f32 = 0.15; // part de la vitesse locale
const ERREUR_VITESSE_MIN: f32 = 0.04; // pixels par minute (~2 km/h)
const ERREUR_CROISSANCE: f32 = 0.05; // dB par minute
const DECALAGE_KM: (f32, f32) = (1.0, 0.1); // km, plus km par minute d'échéance
pub const AROME_SEUIL_MM: f64 = 0.05; // en 15 min (0,2 mm/h, comme le seuil radar)

/// Générateur reproductible (SplitMix64) ; tirages gaussiens par Box-Muller.
pub struct Hasard(pub u64);

impl Hasard {
    fn suivant(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniforme dans ]0, 1[.
    fn uniforme(&mut self) -> f32 {
        ((self.suivant() >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }

    pub fn normal(&mut self) -> f32 {
        let (a, b) = (self.uniforme(), self.uniforme());
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
    }
}

pub struct Ensemble {
    /// [membre][minute 0..=HORIZON] : pluie sur le point.
    pub pluie: Vec<Vec<bool>>,
    /// [membre][minute] : intensité (mm/h).
    pub mm_h: Vec<Vec<f32>>,
}

impl Ensemble {
    pub fn p(&self, t: usize) -> f32 {
        self.pluie.iter().filter(|m| m[t]).count() as f32 / self.pluie.len() as f32
    }

    /// Part des membres mouillés au moins une fois entre les minutes a et b (incluses).
    pub fn p_cumul(&self, a: usize, b: usize) -> f32 {
        self.pluie.iter().filter(|m| m[a..=b].iter().any(|x| *x)).count() as f32 / self.pluie.len() as f32
    }

    /// Intensité médiane des membres mouillés (0 si aucun).
    pub fn mm_h_median(&self, t: usize) -> f32 {
        let mut v: Vec<f32> = self.mm_h.iter().map(|m| m[t]).filter(|x| *x > 0.0).collect();
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    }
}

/// L'ensemble pour le point (x, y) de l'image `g` (la plus récente). `tendance` en dB par minute,
/// `km_px` la taille d'un pixel, `graine` rend le tirage reproductible.
pub fn ensemble(g: &Grille, champ: &Champ, (x, y): (f32, f32), tendance: f32, km_px: f32, graine: u64) -> Ensemble {
    let mut h = Hasard(graine);
    let (u, v) = champ.vitesse(x, y);
    let sigma = (ERREUR_VITESSE * u.hypot(v)).max(ERREUR_VITESSE_MIN);
    let (mut pluie, mut intensite) = (Vec::with_capacity(MEMBRES), Vec::with_capacity(MEMBRES));
    for m in 0..MEMBRES {
        // le membre 0 est la prévision de contrôle, sans perturbation
        let (dv, croissance, e) = if m == 0 {
            ((0.0, 0.0), tendance, (0.0, 0.0))
        } else {
            ((sigma * h.normal(), sigma * h.normal()), tendance + ERREUR_CROISSANCE * h.normal(), (h.normal(), h.normal()))
        };
        let (mut p, mut lp, mut li) = ((x, y), vec![false; HORIZON + 1], vec![0.0; HORIZON + 1]);
        for t in 0..=HORIZON {
            if t > 0 {
                p = champ.reculer(p, 1.0, dv);
            }
            let s = (DECALAGE_KM.0 + DECALAGE_KM.1 * t as f32) / km_px;
            let mut d = g.echantillon(p.0 + e.0 * s, p.1 + e.1 * s);
            if d > SANS {
                d += croissance * t as f32;
            }
            (lp[t], li[t]) = (d >= SEUIL_DBZ, mm_h(d));
        }
        pluie.push(lp);
        intensite.push(li);
    }
    Ensemble { pluie, mm_h: intensite }
}

/// Évolution de l'intensité (dB par minute) dans un carré de 2 × `demi` pixels autour du point :
/// moyenne des dBZ des pixels de pluie, de la première à la dernière image, sur `minutes`.
/// Amortie de moitié et bornée : une tendance ne dure pas.
pub fn tendance(premiere: &Grille, derniere: &Grille, (x, y): (f32, f32), demi: f32, minutes: f32) -> f32 {
    let moyenne = |g: &Grille| {
        let (mut s, mut n) = (0.0f32, 0u32);
        for yy in (y - demi).max(0.0) as usize..((y + demi) as usize).min(g.h) {
            for xx in (x - demi).max(0.0) as usize..((x + demi) as usize).min(g.l) {
                let d = g.dbz[yy * g.l + xx];
                if d >= SEUIL_DBZ {
                    (s, n) = (s + d, n + 1);
                }
            }
        }
        (n >= 50).then(|| s / n as f32)
    };
    match (moyenne(premiere), moyenne(derniere)) {
        (Some(a), Some(b)) if minutes > 0.0 => (0.5 * (b - a) / minutes).clamp(-0.1, 0.1),
        _ => 0.0,
    }
}

/// La prévision AROME autour du point : pour chaque pas de 15 min (heure de fin, secondes),
/// s'il pleut sur chacun des points voisins.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arome {
    pub fins: Vec<i64>,
    pub points: Vec<Vec<bool>>,
}

/// La réponse d'Open-Meteo (un lieu, ou une liste de lieux) → Arome ; None hors du domaine d'AROME.
pub fn arome_de(v: &Value) -> Option<Arome> {
    let lieux: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![v],
        _ => return None,
    };
    let (mut fins, mut points): (Option<Vec<i64>>, Vec<Vec<bool>>) = (None, Vec::new());
    for l in lieux {
        let m = l.get("minutely_15")?;
        let t: Vec<i64> = m.get("time")?.as_array()?.iter().map(Value::as_i64).collect::<Option<_>>()?;
        let p = m.get("precipitation")?.as_array()?;
        if p.len() != t.len() {
            return None;
        }
        if p.iter().all(Value::is_null) {
            continue; // ce point est hors du domaine du modèle
        }
        match &fins {
            None => fins = Some(t),
            Some(f) if *f != t => return None,
            _ => {}
        }
        points.push(p.iter().map(|x| x.as_f64().is_some_and(|mm| mm >= AROME_SEUIL_MM)).collect());
    }
    if points.is_empty() {
        return None;
    }
    Some(Arome { fins: fins?, points })
}

/// Le ciel au point (le point central de la réponse) : code du temps (WMO), nébulosité (%), jour,
/// température, lever et coucher du soleil (secondes). Pour le fond d'écran de la page.
pub fn ciel_de(v: &Value) -> Option<Value> {
    let centre = match v {
        Value::Array(a) => a.get(a.len() / 2)?,
        o => o,
    };
    let c = centre.get("current")?;
    let jour = |k: &str| centre.pointer(&format!("/daily/{k}/0")).cloned().unwrap_or(Value::Null);
    Some(json!({"code": c.get("weather_code"), "nuages": c.get("cloud_cover"), "jour": c.get("is_day").and_then(Value::as_i64).map(|j| j == 1),
        "temperature": c.get("temperature_2m"), "lever": jour("sunrise"), "coucher": jour("sunset")}))
}

impl Arome {
    fn pas(&self, t: i64) -> Option<usize> {
        self.fins.iter().position(|&f| t <= f && t > f - 900)
    }

    fn part(&self, test: impl Fn(&[bool]) -> bool) -> f32 {
        self.points.iter().filter(|p| test(p)).count() as f32 / self.points.len() as f32
    }

    /// Part des points voisins mouillés à l'heure t.
    pub fn p(&self, t: i64) -> Option<f32> {
        let k = self.pas(t)?;
        Some(self.part(|p| p[k]))
    }

    /// Part des points voisins mouillés au moins une fois entre a et b.
    pub fn p_cumul(&self, a: i64, b: i64) -> Option<f32> {
        let (i, j) = (self.pas(a)?, self.pas(b)?);
        Some(self.part(|p| p[i..=j].iter().any(|x| *x)))
    }
}

/// Poids du radar selon l'échéance depuis la dernière image (minutes) : seul jusqu'à 20 min,
/// puis de moins en moins sûr ; AROME prend le reste, la moitié au plus.
pub fn poids_radar(echeance: f32) -> f32 {
    (1.0 - (echeance - 20.0).max(0.0) / 120.0).clamp(0.5, 1.0)
}

pub fn melanger(p_radar: f32, p_arome: Option<f32>, echeance: f32) -> f32 {
    match p_arome {
        Some(a) => {
            let w = poids_radar(echeance);
            w * p_radar + (1.0 - w) * a
        }
        None => p_radar,
    }
}

/// Une minute de la prévision.
#[derive(Clone, Debug, PartialEq)]
pub struct Minute {
    pub t: i64,
    /// Probabilité qu'il pleuve sur le point à cette minute, et d'ici cette minute.
    pub p: f32,
    pub p_cumul: f32,
    pub p_radar: f32,
    pub p_arome: Option<f32>,
    pub mm_h: f32,
}

/// La prévision minute par minute de `maintenant` à +60 min, pour une dernière image prise à t0
/// (secondes). S'arrête à l'horizon de l'ensemble si l'image est ancienne.
pub fn serie(ens: &Ensemble, arome: Option<&Arome>, t0: i64, maintenant: i64) -> Vec<Minute> {
    let debut = ((maintenant - t0) as f32 / 60.0).round().max(0.0) as usize;
    let mut cumul = 0.0f32;
    let mut sortie = Vec::new();
    for k in 0..=60usize {
        let e = debut + k;
        if e > HORIZON {
            break;
        }
        let t = maintenant + 60 * k as i64;
        let (pr, pa) = (ens.p(e), arome.and_then(|a| a.p(t)));
        let p = melanger(pr, pa, e as f32);
        let pc = melanger(ens.p_cumul(debut, e), arome.and_then(|a| a.p_cumul(maintenant, t)), e as f32);
        cumul = cumul.max(pc).max(p); // « d'ici t » ne peut que croître
        sortie.push(Minute { t, p, p_cumul: cumul, p_radar: pr, p_arome: pa, mm_h: ens.mm_h_median(e) });
    }
    sortie
}

#[cfg(test)]
mod essais {
    use super::*;
    use crate::radar::N;

    /// Pluie à 30 dBZ à l'ouest de x = 300, rien à l'est.
    fn front() -> Grille {
        let mut g = Grille::vide(N, N);
        for y in 0..N {
            for x in 0..300 {
                g.dbz[y * N + x] = 30.0;
            }
        }
        g
    }

    #[test]
    fn hasard_gaussien() {
        let mut h = Hasard(7);
        let v: Vec<f32> = (0..20000).map(|_| h.normal()).collect();
        let m = v.iter().sum::<f32>() / v.len() as f32;
        let e = (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / v.len() as f32).sqrt();
        assert!(m.abs() < 0.03 && (e - 1.0).abs() < 0.03, "moyenne {m}, écart {e}");
    }

    #[test]
    fn front_qui_arrive() {
        // le front file vers l'est à 1 pixel par minute (~52 km/h) ; le point est 40 pixels devant
        let ens = ensemble(&front(), &Champ::uniforme(1.0, 0.0), (340.0, 384.0), 0.0, 0.866, 1);
        assert_eq!(ens.pluie.len(), MEMBRES);
        assert!(ens.p(0) == 0.0 && ens.p(20) < 0.1, "trop tôt : {}", ens.p(20));
        assert!(ens.p(70) > 0.9 && ens.p(HORIZON) > 0.9, "après le passage : {}", ens.p(70));
        let moitie = (0..=HORIZON).find(|&t| ens.p(t) >= 0.5).unwrap();
        assert!((35..=45).contains(&moitie), "50 % atteint à {moitie} min");
        assert!(ens.p_cumul(0, 50) >= ens.p(50));
        assert!((ens.mm_h_median(70) / mm_h(30.0) - 1.0).abs() < 0.3, "croissance tirée autour de 0");
        // même tirage, même résultat
        let bis = ensemble(&front(), &Champ::uniforme(1.0, 0.0), (340.0, 384.0), 0.0, 0.866, 1);
        assert_eq!(bis.pluie, ens.pluie);
        // déjà sous la pluie, sans mouvement : il pleut tout du long
        let sous = ensemble(&front(), &Champ::uniforme(0.0, 0.0), (100.0, 384.0), 0.0, 0.866, 2);
        assert!((0..=HORIZON).all(|t| sous.p(t) == 1.0));
        // une pluie qui faiblit vite finit par ne plus mouiller
        let declin = ensemble(&front(), &Champ::uniforme(0.0, 0.0), (100.0, 384.0), -0.4, 0.866, 3);
        assert!(declin.p(0) == 1.0 && declin.p(HORIZON) < 0.2);
    }

    #[test]
    fn tendance_mesuree() {
        let (a, mut b) = (front(), front());
        b.dbz.iter_mut().filter(|d| **d > SANS).for_each(|d| *d = 36.0);
        let t = tendance(&a, &b, (300.0, 384.0), 50.0, 30.0);
        assert!((t - 0.1).abs() < 1e-6, "6 dB en 30 min, amortis puis bornés : {t}");
        assert_eq!(tendance(&a, &b, (700.0, 384.0), 50.0, 30.0), 0.0, "pas de pluie autour");
    }

    fn reponse_arome(t0: i64) -> Value {
        let fins: Vec<i64> = (0..6).map(|k| t0 + 900 * k).collect();
        let mut lieux: Vec<Value> = (0..8)
            .map(|k| json!({"minutely_15": {"time": fins, "precipitation": [0.0, 0.0, if k < 4 { 0.3 } else { 0.0 }, 0.0, 0.1, null]}}))
            .collect();
        lieux.push(json!({"minutely_15": {"time": fins, "precipitation": [null, null, null, null, null, null]}}));
        Value::Array(lieux)
    }

    #[test]
    fn arome_lu_et_melange() {
        let t0 = 1_800_000_000;
        let a = arome_de(&reponse_arome(t0)).unwrap();
        assert_eq!(a.points.len(), 8, "le point hors domaine est ignoré");
        assert_eq!(a.p(t0 + 1800), Some(0.5), "4 points sur 8 dans le pas finissant à t0 + 30 min");
        assert_eq!(a.p(t0 + 1000), Some(0.5));
        assert_eq!(a.p(t0 + 900), Some(0.0));
        assert_eq!(a.p(t0 + 3600), Some(1.0));
        assert_eq!(a.p(t0 + 9000), None);
        assert_eq!(a.p_cumul(t0 + 60, t0 + 1500), Some(0.5));
        assert_eq!(a.p_cumul(t0 + 60, t0 + 3000), Some(1.0));
        assert_eq!(arome_de(&json!([{"minutely_15": {"time": [1, 2], "precipitation": [null, null]}}])), None);
        assert_eq!(arome_de(&json!({"erreur": true})), None);
        // le ciel vient du point central (le 5e des 9)
        let mut r = reponse_arome(t0);
        r[4]["current"] = json!({"weather_code": 3, "cloud_cover": 100, "is_day": 1, "temperature_2m": 13.7});
        r[4]["daily"] = json!({"sunrise": [t0 - 3600], "sunset": [t0 + 30000]});
        assert_eq!(ciel_de(&r), Some(json!({"code": 3, "nuages": 100, "jour": true, "temperature": 13.7, "lever": t0 - 3600, "coucher": t0 + 30000})));
        assert_eq!(ciel_de(&reponse_arome(t0)), None);
        assert_eq!(poids_radar(10.0), 1.0);
        assert_eq!(poids_radar(80.0), 0.5);
        assert_eq!(melanger(0.2, Some(1.0), 80.0), 0.6);
        assert_eq!(melanger(0.2, None, 80.0), 0.2);
    }

    #[test]
    fn serie_depuis_maintenant() {
        let ens = ensemble(&front(), &Champ::uniforme(1.0, 0.0), (340.0, 384.0), 0.0, 0.866, 1);
        let t0 = 1_800_000_000;
        // image prise il y a 12 min : la série commence à la minute 12 de l'ensemble
        let s = serie(&ens, None, t0, t0 + 720);
        assert_eq!(s.len(), 61);
        assert_eq!(s[0].t, t0 + 720);
        assert_eq!(s[30].p, ens.p(42));
        assert!(s.windows(2).all(|w| w[1].p_cumul >= w[0].p_cumul), "le cumul ne décroît pas");
        assert!(s.iter().all(|m| m.p_cumul >= m.p && m.p_arome.is_none()));
        // image vieille de 40 min : on s'arrête à l'horizon
        assert_eq!(serie(&ens, None, t0, t0 + 2400).len(), HORIZON - 40 + 1);
        let a = arome_de(&reponse_arome(t0)).unwrap();
        let s = serie(&ens, Some(&a), t0, t0 + 720);
        assert_eq!(s[0].p_arome, Some(0.0));
        assert!(s.iter().any(|m| m.p_arome == Some(1.0)));
    }
}

//! Une scène : pour la dernière image radar et une mosaïque, tout ce qui ne dépend pas du point —
//! les images récentes, le champ de mouvement corrigé par les cellules suivies, et les images de
//! prévision. Puis la prévision d'un point, en JSON pour la page.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::cellules::{self, Cellule};
use crate::journal::{self, Attente, ECHEANCES};
use crate::mouvement::{self, noeud, Champ};
use crate::prevision::{self, Arome, HORIZON};
use crate::radar::{self, Domaine, Grille, SANS};
use crate::sources::{self, Erreur, Res};

const IMAGES: usize = 4; // 30 min de mouvement
const PAS_IMAGE: usize = 2; // images de prévision réduites de moitié (384 pixels de côté)
const SCENES_MAX: usize = 6;

pub struct Scene {
    pub t0: i64,
    pub domaine: Domaine,
    /// Minutes entre deux images.
    pub dt: f32,
    /// De la plus ancienne à la plus récente (prise à t0).
    pub images: Vec<Grille>,
    pub champ: Champ,
    pub cellules: Vec<Cellule>,
    /// PNG de la pluie transportée, une par échéance de ECHEANCES.
    pub previsions: Vec<Arc<Vec<u8>>>,
}

static SCENES: Mutex<Vec<Arc<Scene>>> = Mutex::new(Vec::new());
static CONSTRUCTION: Mutex<()> = Mutex::new(());

fn trouver(t0: i64, domaine: Domaine) -> Option<Arc<Scene>> {
    SCENES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|s| s.t0 == t0 && s.domaine == domaine).cloned()
}

/// Les dernières heures régulièrement espacées (au plus IMAGES), de la plus ancienne à la plus récente.
pub fn suite_reguliere(heures: &[i64]) -> Vec<i64> {
    let mut suite: Vec<i64> = heures.last().copied().into_iter().collect();
    for &h in heures.iter().rev().skip(1) {
        let ecart = suite[suite.len() - 1] - h;
        let attendu = if suite.len() >= 2 { suite[suite.len() - 2] - suite[suite.len() - 1] } else { ecart };
        if suite.len() == IMAGES || ecart != attendu || !(60..=1800).contains(&ecart) {
            break;
        }
        suite.push(h);
    }
    suite.reverse();
    suite
}

/// La mosaïque de l'image prise à t, tuiles téléchargées en parallèle.
fn mosaique(t: i64, domaine: Domaine) -> Res<Grille> {
    let tuiles: Vec<Res<Grille>> = std::thread::scope(|s| {
        let fils: Vec<_> = domaine
            .tuiles()
            .into_iter()
            .map(|(_, _, x, y)| {
                s.spawn(move || {
                    let o = sources::tuile_radar(t, radar::ZOOM, x as u64, y as u64)?.ok_or(Erreur::Format)?;
                    radar::decoder_tuile(&o).map_err(|_| Erreur::Format)
                })
            })
            .collect();
        fils.into_iter().map(|f| f.join().unwrap_or(Err(Erreur::Format))).collect()
    });
    radar::assembler(&tuiles.into_iter().collect::<Res<Vec<_>>>()?).map_err(|_| Erreur::Format)
}

/// La scène de la dernière image pour ce domaine : gardée, sinon calculée (une seule à la fois).
pub fn scene(domaine: Domaine) -> Res<Arc<Scene>> {
    let suite = suite_reguliere(&sources::heures_radar()?);
    if suite.len() < 2 {
        return Err(Erreur::Source("RainViewer : pas assez d'images récentes".into()));
    }
    let t0 = suite[suite.len() - 1];
    if let Some(s) = trouver(t0, domaine) {
        return Ok(s);
    }
    let _g = CONSTRUCTION.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = trouver(t0, domaine) {
        return Ok(s);
    }
    let images = suite.iter().map(|&t| mosaique(t, domaine)).collect::<Res<Vec<_>>>()?;
    let s = Arc::new(construire(t0, domaine, (suite[1] - suite[0]) as f32 / 60.0, images));
    journal::verifier(t0, domaine, &s.images[s.images.len() - 1]);
    let mut v = SCENES.lock().unwrap_or_else(|e| e.into_inner());
    v.push(s.clone());
    let trop = v.len().saturating_sub(SCENES_MAX);
    v.drain(..trop);
    Ok(s)
}

/// Le calcul d'une scène, sans réseau.
pub fn construire(t0: i64, domaine: Domaine, dt: f32, images: Vec<Grille>) -> Scene {
    let refs: Vec<&Grille> = images.iter().collect();
    let mut champ = mouvement::estimer(&refs, dt);
    let cellules = cellules::suivre(&refs, dt, &champ);
    cellules::corriger(&mut champ, &cellules);
    let previsions = mouvement::transporter(&images[images.len() - 1], &champ, &ECHEANCES, PAS_IMAGE)
        .iter()
        .map(|g| Arc::new(radar::image(g).unwrap_or_default()))
        .collect();
    Scene { t0, domaine, dt, images, champ, cellules, previsions }
}

/// L'image de prévision d'une scène gardée (jamais calculée ici : la page la demande après la prévision).
pub fn image_prevision(t0: i64, tx: u32, ty: u32, echeance: u32) -> Option<Arc<Vec<u8>>> {
    let s = trouver(t0, Domaine { tx, ty })?;
    let k = ECHEANCES.iter().position(|&e| e == echeance)?;
    s.previsions.get(k).cloned()
}

fn r(x: f64, n: i32) -> f64 {
    let f = 10f64.powi(n);
    (x * f).round() / f
}

/// La prévision du point (lat, lon) pour la page.
pub fn prevision(lat: f64, lon: f64, maintenant: i64) -> Res<Value> {
    let s = scene(Domaine::autour(lat, lon))?;
    Ok(decrire(&s, lat, lon, sources::arome(lat, lon).as_ref(), maintenant))
}

/// La prévision d'un point d'une scène, en JSON ; elle attend ensuite sa vérification (journal).
pub fn decrire(s: &Scene, lat: f64, lon: f64, arome: Option<&Arome>, maintenant: i64) -> Value {
    let (x, y) = s.domaine.px(lat, lon);
    let km = radar::km_par_px(lat) as f32;
    let derniere = &s.images[s.images.len() - 1];
    let duree = s.dt * (s.images.len() - 1) as f32;
    let tendance = prevision::tendance(&s.images[0], derniere, (x, y), 100.0 / km, duree);
    let graine = (s.t0 as u64) ^ ((lat * 100.0).round() as i64 as u64).rotate_left(20) ^ ((lon * 100.0).round() as i64 as u64).rotate_left(40);
    let ens = prevision::ensemble(derniere, &s.champ, (x, y), tendance, km, graine);
    let serie = prevision::serie(&ens, arome, s.t0, maintenant);
    let persistance = derniere.part_pluie(x, y, 5.0 / km);

    let mut a = Attente { t0: s.t0, lat, lon, domaine: s.domaine, x, y, p: [0.0; 9], p_radar: [0.0; 9], p_arome: [None; 9],
        p_persist: persistance, faites: 0 };
    for (k, &e) in ECHEANCES.iter().enumerate() {
        a.p_radar[k] = ens.p(e as usize);
        a.p_arome[k] = arome.and_then(|ar| ar.p(s.t0 + 60 * e as i64));
        a.p[k] = prevision::melanger(a.p_radar[k], a.p_arome[k], e as f32);
    }
    journal::noter(a);

    // vitesses en km/h, u vers l'est, v vers le nord
    let kmh = |(u, v): (f32, f32)| (r((u * km * 60.0) as f64, 1), r((-v * km * 60.0) as f64, 1));
    let ll = |px: f32, py: f32| {
        let (la, lo) = s.domaine.lat_lon(px, py);
        json!([r(la, 4), r(lo, 4)])
    };
    let (u, v) = kmh(s.champ.vitesse(x, y));
    let n = s.champ.n;
    let vecteurs: Vec<Value> = (0..n)
        .step_by(2)
        .flat_map(|j| (0..n).step_by(2).map(move |i| (i, j)))
        .map(|(i, j)| {
            let (vu, vv) = kmh((s.champ.u[j * n + i], s.champ.v[j * n + i]));
            json!({"position": ll(noeud(i), noeud(j)), "u": vu, "v": vv})
        })
        .collect();
    let cellules: Vec<Value> = s
        .cellules
        .iter()
        .map(|c| {
            let (cx, cy) = c.position(0.0, &s.champ);
            let (cu, cv) = kmh(c.vitesse.unwrap_or_else(|| s.champ.vitesse(cx, cy)));
            let arrivee = c.arrivee((x, y), HORIZON as u32, &s.champ).map(|t| ((s.t0 + 60 * t as i64 - maintenant) as f64 / 60.0).max(0.0).round());
            json!({"position": ll(cx, cy), "rayon_km": r((c.rayon() * km) as f64, 1), "dbz_max": c.dbz_max, "suivie": c.passe.len(),
                "vitesse_kmh": r(cu.hypot(cv), 0), "passe": c.passe.iter().map(|p| ll(p.0, p.1)).collect::<Vec<_>>(),
                "prevue": (1..=6).map(|k| { let p = c.position(10.0 * k as f32, &s.champ); ll(p.0, p.1) }).collect::<Vec<_>>(),
                "arrivee_min": arrivee})
        })
        .collect();
    let [nord, ouest, sud, est] = s.domaine.bornes();
    let ici = derniere.echantillon(x, y);
    let (xi, yi) = (x.floor() as usize, y.floor() as usize);
    json!({
        "t0": s.t0, "maintenant": maintenant,
        "domaine": {"nord": r(nord, 4), "ouest": r(ouest, 4), "sud": r(sud, 4), "est": r(est, 4)},
        "point": {"lat": lat, "lon": lon, "dbz": (ici > SANS).then_some(ici), "neige": ici > SANS && derniere.neige[yi * derniere.l + xi],
            "persistance": r(persistance as f64, 3)},
        "serie": serie.iter().map(|m| json!({"t": m.t, "p": r(m.p as f64, 3), "p_cumul": r(m.p_cumul as f64, 3), "p_radar": r(m.p_radar as f64, 3),
            "p_arome": m.p_arome.map(|p| r(p as f64, 3)), "mm_h": r(m.mm_h as f64, 2)})).collect::<Vec<_>>(),
        "mouvement": {"vitesse_kmh": r(u.hypot(v), 0), "direction": r((u.atan2(v).to_degrees() + 360.0) % 360.0, 0),
            "qualite": r(s.champ.qualite as f64, 2), "blocs": s.champ.blocs, "tendance_db_h": r((tendance * 60.0) as f64, 1)},
        "vecteurs": vecteurs,
        "cellules": cellules,
        "images": ECHEANCES.iter().map(|&e| json!({"t": s.t0 + 60 * e as i64,
            "url": format!("/prevision/{}/{}/{}/{e}.png", s.t0, s.domaine.tx, s.domaine.ty)})).collect::<Vec<_>>(),
        "arome": arome.is_some(),
    })
}

#[cfg(test)]
mod essais {
    use super::*;
    use crate::mouvement::essais::averses;

    #[test]
    fn suites() {
        assert_eq!(suite_reguliere(&[0, 600, 1200, 1800, 2400, 3000]), vec![1200, 1800, 2400, 3000]);
        assert_eq!(suite_reguliere(&[0, 600, 1800, 2400]), vec![1800, 2400], "trou dans la liste");
        assert_eq!(suite_reguliere(&[2400]), vec![2400]);
        assert_eq!(suite_reguliere(&[]), Vec::<i64>::new());
    }

    #[test]
    fn prevision_d_une_scene() {
        // pluie qui file vers l'est à 1,2 pixel par minute ; le point est devant elle
        let images: Vec<Grille> = (0..4).map(|k| averses(12.0 * k as f32, 0.0)).collect();
        let d = Domaine::autour(44.8065, -0.6303);
        let s = construire(1_800_000_000, d, 10.0, images);
        assert_eq!(s.previsions.len(), 9);
        assert!(s.previsions.iter().all(|p| p.starts_with(b"\x89PNG")));
        assert!((s.champ.global.0 - 1.2).abs() < 0.1, "{:?}", s.champ.global);
        assert!(image_prevision(1_800_000_000, d.tx, d.ty, 30).is_none(), "scène non gardée");
        // un point à l'est de la grande averse (300 + 36, 280) : elle arrive dessus
        let (lat, lon) = d.lat_lon(400.0, 280.0);
        let j = decrire(&s, lat, lon, None, 1_800_000_000 + 600);
        let serie = j["serie"].as_array().unwrap();
        assert_eq!(serie.len(), 61);
        let p = |k: usize| serie[k]["p"].as_f64().unwrap();
        assert!(p(0) < 0.2 && serie.iter().any(|m| m["p"].as_f64().unwrap() > 0.8), "{:?}", serie.iter().map(|m| &m["p"]).collect::<Vec<_>>());
        assert_eq!(serie[60]["p_cumul"].as_f64().unwrap() >= p(60), true);
        let v = j["mouvement"]["vitesse_kmh"].as_f64().unwrap();
        assert!((55.0..70.0).contains(&v), "1,2 px/min × 0,87 km × 60 ≈ 62 km/h : {v}");
        assert!((80.0..100.0).contains(&j["mouvement"]["direction"].as_f64().unwrap()), "vers l'est");
        assert_eq!(j["images"].as_array().unwrap().len(), 9);
        assert_eq!(j["vecteurs"].as_array().unwrap().len(), 144);
        assert_eq!(j["arome"], false);
        assert!(j["cellules"].as_array().unwrap().iter().any(|c| c["suivie"] == 4), "{}", j["cellules"]);
    }
}

//! Journal de vérification : chaque prévision servie est confrontée, image après image, au radar
//! observé ensuite sur le même point (donnees/verification.jsonl, ignoré par git). `pluie --bilan`
//! en tire les scores par échéance, comparés à la persistance (« il pleuvra comme maintenant »).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::calibration::Calibration;
use crate::radar::{Domaine, Grille, SEUIL_DBZ};

/// Échéances vérifiées (minutes après l'image) : une image radar toutes les 10 min.
pub const ECHEANCES: [u32; 9] = [10, 20, 30, 40, 50, 60, 70, 80, 90];
const TAILLE_MAX: u64 = 5 << 20;
const ATTENTES_MAX: usize = 500;

/// Une prévision qui attend les images à venir.
#[derive(Clone, Debug)]
pub struct Attente {
    pub t0: i64,
    pub lat: f64,
    pub lon: f64,
    pub domaine: Domaine,
    pub x: f32,
    pub y: f32,
    /// Aux échéances de ECHEANCES : mélange, radar seul, AROME seul.
    pub p: [f32; 9],
    pub p_radar: [f32; 9],
    pub p_arome: [Option<f32>; 9],
    /// Part de pluie autour du point sur l'image t0.
    pub p_persist: f32,
    pub faites: u16,
}

static ATTENTES: Mutex<Vec<Attente>> = Mutex::new(Vec::new());
/// Le fichier du journal ; non posé (essais du moteur) : rien n'est écrit.
pub static FICHIER: OnceLock<PathBuf> = OnceLock::new();

/// Retient une prévision, une par image et par point (au centième de degré).
/// shortcut: les attentes vivent en mémoire ; un redémarrage perd celles en cours (1 h 30 au plus)
pub fn noter(a: Attente) {
    let mut v = ATTENTES.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|b| b.t0 >= a.t0 - 7200);
    if v.len() < ATTENTES_MAX && !v.iter().any(|b| b.t0 == a.t0 && b.lat == a.lat && b.lon == a.lon) {
        v.push(a);
    }
}

fn arrondi(x: f32) -> f64 {
    (x as f64 * 1000.0).round() / 1000.0
}

/// Confronte les prévisions en attente sur ce domaine à l'image `g` prise à `t` ; rend les lignes écrites.
pub fn verifier(t: i64, domaine: Domaine, g: &Grille) -> Vec<Value> {
    let mut lignes = Vec::new();
    for a in ATTENTES.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().filter(|a| a.domaine == domaine) {
        let Some(k) = ECHEANCES.iter().position(|&e| a.t0 + 60 * e as i64 == t) else { continue };
        if a.faites & (1 << k) != 0 {
            continue;
        }
        a.faites |= 1 << k;
        let obs = g.echantillon(a.x, a.y) >= SEUIL_DBZ;
        lignes.push(json!({"t0": a.t0, "echeance": ECHEANCES[k], "lat": a.lat, "lon": a.lon, "p": arrondi(a.p[k]),
            "p_radar": arrondi(a.p_radar[k]), "p_arome": a.p_arome[k].map(arrondi), "p_persist": arrondi(a.p_persist), "obs": obs as u8}));
    }
    if !lignes.is_empty() {
        ecrire(&lignes);
    }
    lignes
}

fn ecrire(lignes: &[Value]) {
    let Some(f) = FICHIER.get() else { return };
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if std::fs::metadata(f).is_ok_and(|m| m.len() > TAILLE_MAX) {
        // shortcut: deux fichiers au plus (le courant et .1) ; l'ancien .1 est remplacé
        let _ = std::fs::rename(f, f.with_extension("1.jsonl"));
    }
    let texte: String = lignes.iter().map(|l| format!("{l}\n")).collect();
    if let Err(e) = OpenOptions::new().create(true).append(true).open(f).and_then(|mut h| h.write_all(texte.as_bytes())) {
        eprintln!("[!] Journal de vérification : écriture impossible ({e})");
    }
}

#[derive(Default)]
struct Somme {
    n: u32,
    obs: u32,
    b: f64,
    b_radar: f64,
    b_persist: f64,
    b_arome: f64,
    n_arome: u32,
    touches: u32,
    rates: u32,
    fausses: u32,
}

/// Les sommes par échéance des lignes du journal.
fn sommes(texte: &str) -> Vec<Somme> {
    let mut s: Vec<Somme> = ECHEANCES.iter().map(|_| Somme::default()).collect();
    for l in texte.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        let lire = |k: &str| v.get(k).and_then(Value::as_f64);
        let (Some(e), Some(p), Some(pr), Some(pp), Some(o)) = (lire("echeance"), lire("p"), lire("p_radar"), lire("p_persist"), lire("obs")) else { continue };
        let Some(k) = ECHEANCES.iter().position(|&x| x as f64 == e) else { continue };
        let m = &mut s[k];
        m.n += 1;
        m.obs += (o > 0.5) as u32;
        m.b += (p - o).powi(2);
        m.b_radar += (pr - o).powi(2);
        m.b_persist += (pp - o).powi(2);
        if let Some(pa) = lire("p_arome") {
            (m.b_arome, m.n_arome) = (m.b_arome + (pa - o).powi(2), m.n_arome + 1);
        }
        match (p >= 0.5, o > 0.5) {
            (true, true) => m.touches += 1,
            (false, true) => m.rates += 1,
            (true, false) => m.fausses += 1,
            _ => {}
        }
    }
    s
}

/// Le bilan des lignes du journal : scores de Brier par échéance (plus bas = mieux), gain sur la
/// persistance, détection et fausses alertes au seuil de 50 %.
pub fn bilan(texte: &str) -> String {
    let s = sommes(texte);
    let total: u32 = s.iter().map(|m| m.n).sum();
    if total == 0 {
        return "Aucune prévision vérifiée pour l'instant : le journal se remplit quand la page est ouverte.".into();
    }
    let pc = |a: u32, b: u32| if b == 0 { "   —".to_string() } else { format!("{:>3.0}%", 100.0 * a as f64 / b as f64) };
    let mut r = format!("Bilan de fiabilité : {total} vérifications (pluie = 12 dBZ, ~0,2 mm/h ; Brier : plus bas = mieux)\n");
    r += "éch.       n  pluie  mélange  radar  AROME  persist.  gain   détect.  fausses\n";
    for (k, m) in s.iter().enumerate().filter(|(_, m)| m.n > 0) {
        let n = m.n as f64;
        let arome = if m.n_arome > 0 { format!("{:.3}", m.b_arome / m.n_arome as f64) } else { "  —  ".into() };
        let gain = if m.b_persist > 0.0 { format!("{:+.2}", 1.0 - m.b / m.b_persist) } else { "  — ".into() };
        r += &format!("{:>2} min {:>6}  {}   {:.3}  {:.3}  {arome}  {:.3}    {gain}   {}     {}\n", ECHEANCES[k], m.n, pc(m.obs, m.n),
            m.b / n, m.b_radar / n, m.b_persist / n, pc(m.touches, m.touches + m.rates), pc(m.fausses, m.touches + m.fausses));
    }
    r + "gain : 1 − Brier / Brier de la persistance (positif = mieux que « il pleuvra comme maintenant »)."
}

/// Le même bilan en JSON, pour la page.
pub fn bilan_json(texte: &str) -> Value {
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    let part = |a: u32, b: u32| (b > 0).then(|| r(a as f64 / b as f64));
    let s = sommes(texte);
    let echeances: Vec<Value> = s
        .iter()
        .enumerate()
        .filter(|(_, m)| m.n > 0)
        .map(|(k, m)| {
            let n = m.n as f64;
            json!({"echeance": ECHEANCES[k], "n": m.n, "pluie": part(m.obs, m.n), "brier": r(m.b / n), "brier_radar": r(m.b_radar / n),
                "brier_arome": (m.n_arome > 0).then(|| r(m.b_arome / m.n_arome as f64)), "brier_persist": r(m.b_persist / n),
                "gain": (m.b_persist > 0.0).then(|| r(1.0 - m.b / m.b_persist)), "detection": part(m.touches, m.touches + m.rates),
                "fausses": part(m.fausses, m.touches + m.fausses)})
        })
        .collect();
    json!({"total": s.iter().map(|m| m.n).sum::<u32>(), "echeances": echeances})
}

/// Tout le journal sur disque : l'ancien fichier .1, puis le courant.
fn lire_tout(f: &Path) -> String {
    [f.with_extension("1.jsonl"), f.to_path_buf()].iter().filter_map(|c| std::fs::read_to_string(c).ok()).collect::<Vec<_>>().join("\n")
}

pub fn bilan_fichier(f: &Path) -> String {
    format!("{}\n(journal : {})", bilan(&lire_tout(f)), f.display())
}

/// Le réglage automatique des probabilités, relu dans le journal au plus une fois par heure
/// (journal non posé, dans les essais : aucun réglage).
pub fn calibration() -> Arc<Calibration> {
    static GARDE: Mutex<Option<(Instant, Arc<Calibration>)>> = Mutex::new(None);
    let mut g = GARDE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((quand, c)) = g.as_ref() {
        if quand.elapsed() < Duration::from_secs(3600) {
            return c.clone();
        }
    }
    let c = Arc::new(FICHIER.get().map(|f| Calibration::depuis(&lire_tout(f))).unwrap_or_default());
    *g = Some((Instant::now(), c.clone()));
    c
}

/// La fiabilité pour la page : le bilan et le réglage en cours.
pub fn fiabilite() -> Value {
    let mut v = bilan_json(&FICHIER.get().map(|f| lire_tout(f)).unwrap_or_default());
    v["reglage"] = calibration().resume();
    v
}

#[cfg(test)]
mod essais {
    use super::*;
    use crate::radar::N;

    #[test]
    fn attente_verifiee_une_fois() {
        let d = Domaine { tx: 63, ty: 46 };
        let t0 = 1_900_000_000;
        let mut a = Attente { t0, lat: 44.81, lon: -0.63, domaine: d, x: 100.5, y: 200.5, p: [0.5; 9], p_radar: [0.4; 9],
            p_arome: [None; 9], p_persist: 0.0, faites: 0 };
        a.p[1] = 0.9;
        a.p_arome[1] = Some(0.25);
        noter(a.clone());
        noter(a.clone()); // la même : ignorée
        let mut g = Grille::vide(N, N);
        g.dbz[200 * N + 100] = 30.0;
        assert!(verifier(t0 + 1200, Domaine { tx: 1, ty: 1 }, &g).is_empty(), "autre domaine");
        assert!(verifier(t0 + 1500, d, &g).is_empty(), "pas une échéance");
        let l = verifier(t0 + 1200, d, &g);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0]["echeance"], 20);
        assert_eq!(l[0]["obs"], 1);
        assert_eq!(l[0]["p"], 0.9);
        assert_eq!(l[0]["p_arome"], 0.25);
        assert!(verifier(t0 + 1200, d, &g).is_empty(), "déjà vérifiée");
    }

    #[test]
    fn bilan_lisible() {
        assert!(bilan("").starts_with("Aucune prévision"));
        let texte = [
            r#"{"echeance":10,"p":0.9,"p_radar":0.9,"p_arome":null,"p_persist":1.0,"obs":1}"#,
            r#"{"echeance":10,"p":0.2,"p_radar":0.1,"p_arome":0.5,"p_persist":1.0,"obs":0}"#,
            "pas du json",
            r#"{"echeance":30,"p":0.6,"p_radar":0.6,"p_persist":0.0,"obs":0}"#,
        ]
        .join("\n");
        let b = bilan(&texte);
        assert!(b.starts_with("Bilan de fiabilité : 3 vérifications"), "{b}");
        let l10 = b.lines().find(|l| l.starts_with("10 min")).unwrap();
        // Brier mélange (0,01 + 0,04) / 2 = 0,025 ; persistance (0 + 1) / 2 = 0,5 ; gain 0,95
        assert!(l10.contains("0.025") && l10.contains("0.500") && l10.contains("+0.95"), "{l10}");
        assert!(l10.contains(" 50%"), "pluie observée une fois sur deux : {l10}");
        let l30 = b.lines().find(|l| l.starts_with("30 min")).unwrap();
        assert!(l30.contains("100%"), "une fausse alerte sur une : {l30}");
        let j = bilan_json(&texte);
        assert_eq!(j["total"], 3);
        assert_eq!(j["echeances"][0]["echeance"], 10);
        assert_eq!(j["echeances"][0]["brier"], 0.025);
        assert_eq!(j["echeances"][0]["gain"], 0.95);
        assert_eq!(j["echeances"][0]["brier_arome"], 0.25);
        assert_eq!(j["echeances"][1]["detection"], Value::Null, "aucune pluie observée à 30 min");
        assert_eq!(j["echeances"][1]["fausses"], 1.0);
    }
}

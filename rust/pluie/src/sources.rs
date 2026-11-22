//! Les sources externes, relayées et gardées en cache (repris de web_weather) :
//!   - RainViewer (images radar des 2 dernières heures, toutes les 10 min) ;
//!   - les tuiles OpenStreetMap (fond de carte, cache disque) ;
//!   - Open-Meteo (AROME au pas de 15 min autour du point, et le ciel actuel pour le fond d'écran) ;
//!   - l'API Adresse (nom de la commune, position arrondie à ~1 km).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Map, Value};
use web_tools_commun::serveur::{encoder_url, params};

use crate::radar::ZOOM;

pub const AGENT: &str = "web_pluie/1.0 (+https://github.com/pilou33620/web_pluie)";

pub const OM: &str = "https://api.open-meteo.com/v1/forecast";
pub const RAINVIEWER: &str = "https://api.rainviewer.com/public/weather-maps.json";
pub const OSM: &str = "https://tile.openstreetmap.org";
pub const BAN: &str = "https://api-adresse.data.gouv.fr/reverse/";

pub const OSM_ZOOM_MAX: u32 = 13;
const OSM_DUREE: u64 = 7 * 86400; // une tuile OSM gardée 7 jours

#[derive(Debug, PartialEq)]
pub enum Erreur {
    /// Paramètre de la page refusé (400).
    Parametre,
    /// Une source externe n'a pas répondu : message court, sans chemin disque (502).
    Source(String),
    /// Une source a changé de format : le dire, plutôt qu'accuser la page (502).
    Format,
}

pub type Res<T> = Result<T, Erreur>;

/// La racine de l'outil (web/, cache/, donnees/) : le dossier courant s'il contient la page, sinon
/// le premier dossier au-dessus de l'exécutable qui la contient.
pub fn racine() -> &'static Path {
    static R: OnceLock<PathBuf> = OnceLock::new();
    R.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let outil = |d: &Path| d.join("web").join("index.html").is_file() && d.join("web").join("js").join("app.js").is_file();
        if outil(&cwd) {
            return cwd;
        }
        let exe = std::env::current_exe().ok().and_then(|e| std::fs::canonicalize(e).ok());
        exe.and_then(|e| e.ancestors().find(|d| outil(d)).map(Path::to_path_buf)).unwrap_or(cwd)
    })
}

/// Le cache disque des tuiles OSM (ignoré par git).
pub static CACHE_DISQUE: OnceLock<PathBuf> = OnceLock::new();

fn cache_disque() -> &'static Path {
    CACHE_DISQUE.get_or_init(|| racine().join("cache"))
}

// --- Appels sortants et cache mémoire ------------------------------------------

/// Remplace les sources dans les essais (aucun réseau).
#[cfg(test)]
pub static FAUX: Mutex<Option<fn(&str) -> Res<Vec<u8>>>> = Mutex::new(None);

fn hote_de(url: &str) -> &str {
    let reste = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    reste.split(['/', '?', ':']).next().unwrap_or("")
}

pub fn telecharger(url: &str, max_octets: u64) -> Res<Vec<u8>> {
    #[cfg(test)]
    {
        let faux = *FAUX.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = faux {
            return f(url);
        }
    }
    static CLIENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        let d = Duration::from_secs(20);
        ureq::AgentBuilder::new().timeout_connect(d).timeout_read(d).timeout_write(d).user_agent(AGENT).build()
    });
    let hote = hote_de(url);
    let rep = match CLIENT.get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err(Erreur::Source(format!("{hote} : HTTP {code}"))),
        Err(_) => return Err(Erreur::Source(format!("{hote} injoignable"))),
    };
    let mut corps = Vec::new();
    rep.into_reader()
        .take(max_octets + 1)
        .read_to_end(&mut corps)
        .map_err(|_| Erreur::Source(format!("{hote} injoignable")))?;
    if corps.len() as u64 > max_octets {
        return Err(Erreur::Source(format!("{hote} : réponse trop grande")));
    }
    Ok(corps)
}

fn lire_json(corps: &[u8], hote: &str) -> Res<Value> {
    serde_json::from_slice(corps).map_err(|_| Erreur::Source(format!("{hote} : réponse illisible")))
}

fn json_de(url: &str, p: &[(&str, &str)]) -> Res<Value> {
    let q: Vec<String> = p.iter().map(|(k, v)| format!("{}={}", encoder_url(k), encoder_url(v))).collect();
    lire_json(&telecharger(&format!("{url}?{}", q.join("&")), 8 << 20)?, hote_de(url))
}

#[derive(Clone)]
pub enum Valeur {
    Json(Arc<Value>),
    Octets(Arc<Vec<u8>>),
}

pub const CACHE_MAX: usize = 1500; // shortcut: borne en nombre d'entrées, pas en octets ; à revoir si la mémoire du Pi souffre

type Table = (HashMap<String, (Instant, u64, Valeur)>, u64); // (fin, rang d'insertion, valeur), compteur
static CACHE: LazyLock<Mutex<Table>> = LazyLock::new(|| Mutex::new((HashMap::new(), 0)));
static EN_COURS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// La valeur gardée si elle a moins de `duree` secondes, sinon calcul() (les erreurs ne se gardent pas).
/// Un seul calcul à la fois par clé : deux appareils qui demandent la même tuile ne coûtent qu'un appel.
pub fn en_cache(cle: &str, duree: u64, calcul: impl FnOnce() -> Res<Valeur>) -> Res<Valeur> {
    let verrou = EN_COURS.lock().unwrap_or_else(|e| e.into_inner()).entry(cle.to_string()).or_default().clone();
    let _g = verrou.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((fin, _, v)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).0.get(cle) {
        if *fin > Instant::now() {
            return Ok(v.clone());
        }
    }
    let valeur = calcul()?;
    let mut t = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let (table, compteur) = &mut *t;
    if table.len() >= CACHE_MAX {
        // les plus anciennes entrées d'abord
        let mut rangs: Vec<(u64, String)> = table.iter().map(|(k, (_, n, _))| (*n, k.clone())).collect();
        rangs.sort();
        let mut en_cours = EN_COURS.lock().unwrap_or_else(|e| e.into_inner());
        for (_, k) in rangs.into_iter().take(CACHE_MAX / 2) {
            table.remove(&k);
            en_cours.remove(&k);
        }
    }
    *compteur += 1; // réinsérée avec un rang neuf : l'ordre suit la fraîcheur
    table.insert(cle.to_string(), (Instant::now() + Duration::from_secs(duree), *compteur, valeur.clone()));
    Ok(valeur)
}

pub fn json_en_cache(cle: &str, duree: u64, calcul: impl FnOnce() -> Res<Value>) -> Res<Arc<Value>> {
    match en_cache(cle, duree, || calcul().map(|v| Valeur::Json(Arc::new(v))))? {
        Valeur::Json(v) => Ok(v),
        Valeur::Octets(_) => Err(Erreur::Format),
    }
}

fn octets_en_cache(cle: &str, duree: u64, calcul: impl FnOnce() -> Res<Vec<u8>>) -> Res<Arc<Vec<u8>>> {
    match en_cache(cle, duree, || calcul().map(|o| Valeur::Octets(Arc::new(o))))? {
        Valeur::Octets(o) => Ok(o),
        Valeur::Json(_) => Err(Erreur::Format),
    }
}

/// Oublie une entrée (essais).
pub fn oublier(cle: &str) {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).0.remove(cle);
}

// --- Petites aides de lecture -----------------------------------------------------

/// Première valeur non vide du paramètre.
pub fn param(q: &str, nom: &str) -> Option<String> {
    params(q).into_iter().find(|(k, v)| k == nom && !v.is_empty()).map(|(_, v)| v)
}

fn arrondi(x: f64, n: i32) -> f64 {
    let f = 10f64.powi(n);
    (x * f).round_ties_even() / f
}

/// (lat, lon) arrondis à `pas`, ou Parametre. Un lieu arrondi partage son cache.
pub fn coordonnees(q: &str, pas: f64) -> Res<(f64, f64)> {
    let lire = |n: &str| param(q, n).and_then(|v| v.trim().parse::<f64>().ok()).ok_or(Erreur::Parametre);
    let (lat, lon) = (lire("lat")?, lire("lon")?);
    if !(lat.is_finite() && lon.is_finite() && (-85.0..=85.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
        return Err(Erreur::Parametre);
    }
    let a = |x: f64| arrondi((x / pas).round_ties_even() * pas, 4);
    Ok((a(lat), a(lon)))
}

/// Le déplacement envoyé par la page (`vitesse` en km/h, `cap` en degrés, 0 = nord) : None à
/// l'arrêt (moins de 5 km/h) ou sans ces paramètres ; Parametre si l'un est hors bornes.
pub fn deplacement(q: &str) -> Res<Option<(f64, f64)>> {
    let lire = |n: &str| param(q, n).map(|v| v.trim().parse::<f64>().ok().filter(|x| x.is_finite()).ok_or(Erreur::Parametre)).transpose();
    let (vitesse, cap) = (lire("vitesse")?, lire("cap")?);
    let Some(v) = vitesse else { return Ok(None) };
    if !(0.0..=250.0).contains(&v) || cap.is_some_and(|c| !(0.0..=360.0).contains(&c)) {
        return Err(Erreur::Parametre);
    }
    Ok(match cap {
        Some(c) if v >= 5.0 => Some((v, c)),
        _ => None,
    })
}

/// Le nom de la commune (API Adresse, France seulement), gardé 30 jours ; None ailleurs ou en panne.
pub fn commune(lat: f64, lon: f64) -> Option<String> {
    let (la, lo) = (arrondi(lat, 2).to_string(), arrondi(lon, 2).to_string());
    let v = json_en_cache(&format!("commune|{la}|{lo}"), 30 * 86400, || json_de(BAN, &[("lat", la.as_str()), ("lon", lo.as_str())])).ok()?;
    v.pointer("/features/0/properties/city").and_then(Value::as_str).map(String::from)
}

// --- Radar --------------------------------------------------------------------------

/// {"host": …, "images": {heure: chemin}} de RainViewer, gardé 2 min.
fn radar_brut() -> Res<Arc<Value>> {
    json_en_cache("radar", 120, || {
        let r = lire_json(&telecharger(RAINVIEWER, 1 << 20)?, "api.rainviewer.com")?;
        let host = r.get("host").and_then(Value::as_str).ok_or(Erreur::Format)?;
        let mut images = Map::new();
        for f in r.pointer("/radar/past").and_then(Value::as_array).ok_or(Erreur::Format)? {
            let t = f.get("time").and_then(Value::as_i64).ok_or(Erreur::Format)?;
            let chemin = f.get("path").and_then(Value::as_str).ok_or(Erreur::Format)?;
            images.insert(t.to_string(), json!(chemin));
        }
        Ok(json!({"host": host, "images": images}))
    })
}

/// Les heures des images radar disponibles, de la plus ancienne à la plus récente.
pub fn heures_radar() -> Res<Vec<i64>> {
    let r = radar_brut()?;
    let mut h: Vec<i64> = r["images"].as_object().map(|m| m.keys().filter_map(|k| k.parse().ok()).collect()).unwrap_or_default();
    h.sort_unstable();
    Ok(h)
}

pub fn radar() -> Res<Arc<Value>> {
    Ok(Arc::new(json!({"images": heures_radar()?, "zoom_max": ZOOM})))
}

fn dans_la_carte(z: u32, x: u64, y: u64) -> bool {
    x < 1 << z && y < 1 << z
}

/// Tuile radar brute (palette Universal Blue, sans lissage, neige à part) : la même sert à la carte
/// et au calcul, ce qui épargne les appels (RainViewer : 100 par minute au plus).
pub fn tuile_radar(t: i64, z: u32, x: u64, y: u64) -> Res<Option<Arc<Vec<u8>>>> {
    if z > ZOOM || !dans_la_carte(z, x, y) {
        return Ok(None);
    }
    let r = radar_brut()?;
    // Le chemin vient de la liste de RainViewer : la page ne choisit qu'une heure de cette liste
    let Some(chemin) = r["images"].get(t.to_string()).and_then(Value::as_str) else { return Ok(None) };
    let url = format!("{}{chemin}/256/{z}/{x}/{y}/2/0_1.png", r["host"].as_str().unwrap_or(""));
    octets_en_cache(&url, 7200, || telecharger(&url, 1 << 20)).map(Some)
}

/// Tuile OSM depuis le cache disque (7 jours), sinon téléchargée puis rangée.
/// Si OSM ne répond pas, une tuile périmée vaut mieux que rien.
pub fn tuile_osm(z: u32, x: u64, y: u64) -> Res<Option<Arc<Vec<u8>>>> {
    if z > OSM_ZOOM_MAX || !dans_la_carte(z, x, y) {
        return Ok(None);
    }
    let fichier = cache_disque().join("osm").join(z.to_string()).join(x.to_string()).join(format!("{y}.png"));
    let age = std::fs::metadata(&fichier).and_then(|m| m.modified()).ok().and_then(|m| SystemTime::now().duration_since(m).ok());
    if age.is_some_and(|a| a.as_secs() < OSM_DUREE) {
        if let Ok(o) = std::fs::read(&fichier) {
            return Ok(Some(Arc::new(o)));
        }
    }
    let corps = match telecharger(&format!("{OSM}/{z}/{x}/{y}.png"), 1 << 20) {
        Ok(c) => c,
        Err(e) => {
            if let Ok(o) = std::fs::read(&fichier) {
                return Ok(Some(Arc::new(o)));
            }
            return Err(e);
        }
    };
    // shortcut: le cache disque n'est jamais purgé ; quelques dizaines de Mo autour d'un lieu, à purger si on voyage beaucoup
    static NUMERO: AtomicU64 = AtomicU64::new(0);
    let tmp = fichier.with_extension(format!("png.{}.{}.tmp", std::process::id(), NUMERO.fetch_add(1, Ordering::SeqCst)));
    let ecrit = fichier
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&tmp, &corps))
        .and_then(|_| std::fs::rename(&tmp, &fichier));
    if ecrit.is_err() {
        let _ = std::fs::remove_file(&tmp); // disque plein : la tuile sert quand même, sans rester tronquée
    }
    Ok(Some(Arc::new(corps)))
}

// --- Open-Meteo autour du point ----------------------------------------------------

/// AROME (France, 2,5 km) sur 9 points autour du lieu (~10 km) : pluie au pas de 15 min, et pour le
/// fond d'écran le ciel actuel (code du temps, nébulosité, jour, température, lever et coucher du
/// soleil). Gardé 15 min (le modèle tourne toutes les heures). None si Open-Meteo ne répond pas.
pub fn autour(lat: f64, lon: f64) -> Option<Arc<Value>> {
    let (lat, lon) = (arrondi((lat / 0.05).round() * 0.05, 2), arrondi((lon / 0.05).round() * 0.05, 2));
    let mut las = Vec::new();
    let mut los = Vec::new();
    for i in -1..=1 {
        for j in -1..=1 {
            las.push(arrondi(lat + 0.09 * i as f64, 3).to_string());
            los.push(arrondi(lon + 0.13 * j as f64, 3).to_string());
        }
    }
    let (las, los) = (las.join(","), los.join(","));
    json_en_cache(&format!("autour|{lat}|{lon}"), 900, || {
        json_de(OM, &[("latitude", las.as_str()), ("longitude", los.as_str()), ("models", "meteofrance_arome_france"),
            ("minutely_15", "precipitation"), ("forecast_minutely_15", "8"), ("past_minutely_15", "1"),
            ("current", "weather_code,cloud_cover,is_day,temperature_2m"), ("daily", "sunrise,sunset"), ("forecast_days", "1"),
            ("timezone", "auto"), ("timeformat", "unixtime")])
    })
    .ok()
}

#[cfg(test)]
mod essais {
    use super::*;

    #[test]
    fn coordonnees_arrondies() {
        assert_eq!(coordonnees("lat=44.80565&lon=-0.6324", 0.01), Ok((44.81, -0.63)));
        assert_eq!(coordonnees("lat=44.84&lon=-0.66", 0.1), Ok((44.8, -0.7)));
        for q in ["", "lat=abc&lon=1", "lat=86&lon=0", "lat=0&lon=181", "lat=nan&lon=0", "lat=inf&lon=0"] {
            assert_eq!(coordonnees(q, 0.01), Err(Erreur::Parametre), "{q}");
        }
    }

    #[test]
    fn deplacements() {
        assert_eq!(deplacement(""), Ok(None));
        assert_eq!(deplacement("vitesse=45&cap=270"), Ok(Some((45.0, 270.0))));
        assert_eq!(deplacement("vitesse=3&cap=270"), Ok(None), "à l'arrêt");
        assert_eq!(deplacement("vitesse=45"), Ok(None), "sans cap");
        for q in ["vitesse=abc&cap=1", "vitesse=45&cap=361", "vitesse=-1&cap=0", "vitesse=900&cap=0", "vitesse=inf&cap=0", "vitesse=45&cap=nan"] {
            assert_eq!(deplacement(q), Err(Erreur::Parametre), "{q}");
        }
    }

    #[test]
    fn hotes() {
        assert_eq!(hote_de("https://api.open-meteo.com/v1/forecast?x=1"), "api.open-meteo.com");
        assert_eq!(hote_de("https://tiles.exemple:8443/a"), "tiles.exemple");
    }
}

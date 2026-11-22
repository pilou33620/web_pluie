//! Banc du serveur : ce qu'il sert, ce qu'il refuse, et la prévision de bout en bout, sans réseau.
//! Le serveur démarre sur un port libre de 127.0.0.1. Les sources (RainViewer, OSM, Open-Meteo)
//! sont remplacées par de fausses réponses : une averse synthétique qui file vers l'est.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::LazyLock;

use serde_json::{json, Value};

use super::*;
use crate::radar::{self, Grille, TUILE};
use crate::sources::{BAN, CACHE_DISQUE, FAUX, OM, OSM, RAINVIEWER};

/// L'heure de la dernière fausse image : il y a 10 à 20 min, comme chez RainViewer.
static T: LazyLock<i64> = LazyLock::new(|| maintenant() / 600 * 600 - 600);
const HOTE: &str = "https://tiles.exemple";

/// Une averse de 40 dBZ, 30 pixels de rayon, à 60 pixels à l'ouest de Pessac à l'heure T, qui file
/// vers l'est à 1 pixel par minute.
fn fausse_tuile(t: i64, x: i64, y: i64) -> Vec<u8> {
    let (px, py) = radar::monde_px(44.8065, -0.6303);
    let cx = px - 60.0 + (t - *T) as f64 / 60.0;
    let mut g = Grille::vide(TUILE, TUILE);
    for j in 0..TUILE {
        for i in 0..TUILE {
            let d = ((x as f64 * 256.0 + i as f64 + 0.5 - cx).powi(2) + (y as f64 * 256.0 + j as f64 + 0.5 - py).powi(2)).sqrt();
            if d < 30.0 {
                g.dbz[j * TUILE + i] = 40.0;
            }
        }
    }
    radar::image(&g).unwrap()
}

fn faux(url: &str) -> Result<Vec<u8>, Erreur> {
    let ok = |v: Value| -> Result<Vec<u8>, Erreur> { Ok(v.to_string().into_bytes()) };
    if url == RAINVIEWER {
        let past: Vec<Value> = (0..4).rev().map(|k| *T - 600 * k).map(|t| json!({"time": t, "path": format!("/v2/radar/{t}")})).collect();
        return ok(json!({"host": HOTE, "radar": {"past": past}}));
    }
    if let Some(reste) = url.strip_prefix(&format!("{HOTE}/v2/radar/")) {
        // <t>/256/<z>/<x>/<y>/2/0_1.png
        let p: Vec<&str> = reste.split('/').collect();
        assert_eq!((p[1], p[5], p[6]), ("256", "2", "0_1.png"), "tuile brute, palette Universal Blue : {url}");
        let n = |k: usize| p[k].parse::<i64>().unwrap();
        if n(2) != 7 {
            return Ok(radar::image(&Grille::vide(TUILE, TUILE)).unwrap());
        }
        return Ok(fausse_tuile(n(0), n(3), n(4)));
    }
    if url.starts_with(BAN) {
        return ok(json!({"features": [{"properties": {"city": "Pessac"}}]}));
    }
    if url.starts_with(OSM) {
        return Ok(b"PNG-OSM".to_vec());
    }
    if url.starts_with(OM) {
        assert!(url.contains("models=meteofrance_arome_france&") && url.contains("minutely_15=precipitation"), "{url}");
        let fins: Vec<i64> = (-1..8).map(|k| maintenant() / 900 * 900 + 900 * k).collect();
        let lieu = json!({"minutely_15": {"time": fins, "precipitation": vec![0.0; 9]},
            "current": {"weather_code": 2, "cloud_cover": 40, "is_day": 1, "temperature_2m": 12.5}, "daily": {"sunrise": [1], "sunset": [2]}});
        return ok(Value::Array(vec![lieu; 9]));
    }
    Err(Erreur::Source("source.exemple injoignable".into()))
}

struct Rep {
    code: u16,
    tete: String,
    corps: Vec<u8>,
}

impl Rep {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.corps).unwrap_or(Value::Null)
    }
}

fn requete(port: u16, chemin: &str, hote: Option<&str>, methode: &str) -> Rep {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let hote = hote.map(String::from).unwrap_or_else(|| format!("127.0.0.1:{port}"));
    write!(s, "{methode} {chemin} HTTP/1.0\r\nHost: {hote}\r\n\r\n").unwrap();
    let mut brut = Vec::new();
    let _ = s.read_to_end(&mut brut);
    let fin = brut.windows(4).position(|w| w == b"\r\n\r\n").expect("réponse sans en-têtes");
    let tete = String::from_utf8_lossy(&brut[..fin]).into_owned();
    let code = tete.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Rep { code, tete, corps: brut[fin + 4..].to_vec() }
}

#[test]
fn ligne_de_commande() {
    let a = |l: &[&str]| l.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // la commande du catalogue web_launcher
    let x = lire_args(&a(&["--port", "8802", "--sans-navigateur", "--sans-pause", "--sans-maj"])).unwrap();
    assert_eq!((x.port, x.sans_navigateur, x.sans_pause, x.sans_maj, x.local, x.bilan), (8802, true, true, true, false, false));
    assert_eq!(lire_args(&[]).unwrap().port, PORT);
    assert!(lire_args(&a(&["--local"])).unwrap().local);
    assert!(lire_args(&a(&["--bilan"])).unwrap().bilan);
    assert_eq!(lire_args(&a(&["--port=9000"])).unwrap().port, 9000);
    assert!(lire_args(&a(&["--port"])).is_err());
    assert!(lire_args(&a(&["--port", "abc"])).is_err());
    assert!(lire_args(&a(&["--inconnu"])).is_err());
    assert!(lire_args(&a(&["--local=1"])).is_err());
}

#[test]
fn chemins_reconnus() {
    assert_eq!(tuile("/tuiles/osm/7/63/45.png"), Some((None, 7, 63, 45)));
    assert_eq!(tuile("/tuiles/radar/1793209800/6/31/22.png"), Some((Some(1793209800), 6, 31, 22)));
    for c in ["/tuiles/osm/../../Cargo.toml", "/tuiles/osm/123/0/0.png", "/tuiles/radar/12/1/0/0.png", "/tuiles/osm/1/0/0.png/x"] {
        assert_eq!(tuile(c), None, "{c}");
    }
    assert_eq!(image_prevision("/prevision/1793209800/63/46/30.png"), Some((1793209800, 63, 46, 30)));
    for c in ["/prevision/1793209800/63/46.png", "/prevision/1793209800/63/46/1000.png", "/prevision/x/63/46/30.png", "/prevision/1793209800/63/46/30.png/.."] {
        assert_eq!(image_prevision(c), None, "{c}");
    }
}

#[test]
fn banc_serveur() {
    let tmp: PathBuf = std::env::temp_dir().join(format!("web_pluie_essai_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(CACHE_DISQUE.set(tmp.join("cache")).is_ok());
    *FAUX.lock().unwrap() = Some(faux);
    let _ = GARDE.set(Garde::new("WEB_PLUIE", false, None, None));
    let srv = Serveur::ouvrir(0, false, 1).unwrap();
    srv.servir_en_fond(Arc::new(repondre));
    let port = srv.port();
    let get = |c: &str| requete(port, c, None, "GET");

    // --- Fichiers servis et refusés
    for (c, attendu) in [
        ("/", 200), ("/index.html", 200), ("/sw.js", 200), ("/manifest.json", 200), ("/js/app.js", 200), ("/js/paysage.js", 200), ("/css/style.css", 200),
        ("/css/icone.svg", 200), ("/css/icone-180.png", 200), ("/css/icone-512.png", 200), ("/vendor/leaflet/leaflet.js", 200), ("/vendor/leaflet/leaflet.css", 200), ("/favicon.ico", 204),
        ("/.git/config", 404), ("/.gitignore", 404), ("/Cargo.toml", 404), ("/rust/pluie/src/serveur.rs", 404), ("/README.md", 404),
        ("/web/index.html", 404), ("/js/", 404), ("/js/../Cargo.toml", 404), ("/js/%2e%2e/Cargo.toml", 404), ("/js/.cache.js", 404),
        ("/vendor/leaflet", 404), ("/cache/osm/1/0/0.png", 404), ("/donnees/verification.jsonl", 404), ("/C:%5cWindows%5cwin.ini", 404),
        ("/api/inconnue", 404), ("/prevision/1793209800/63/46/30.png", 404),
    ] {
        assert_eq!(get(c).code, attendu, "GET {c}");
    }
    assert_eq!(requete(port, "/.git/config", None, "HEAD").code, 404);
    for (c, t) in [("/", "text/html; charset=utf-8"), ("/js/app.js", "text/javascript; charset=utf-8"), ("/css/style.css", "text/css; charset=utf-8"),
        ("/manifest.json", "application/json"), ("/css/icone.svg", "image/svg+xml"), ("/css/icone-192.png", "image/png")] {
        let r = get(c);
        assert_eq!(r.code, 200, "{c}");
        assert!(r.tete.lines().any(|l| l == format!("Content-Type: {t}")), "{c} : {}", r.tete);
    }
    for inconnu in ["LICENSE", "a.txt", "a.exe", "a.md", "a.jsonl"] {
        assert_eq!(type_mime(Path::new(inconnu)), None, "{inconnu}");
    }
    assert_eq!(requete(port, "/", Some("evil.example:8802"), "GET").code, 403, "Host étranger");
    assert_eq!(requete(port, "/api/prevision?lat=44.8&lon=-0.6", Some("evil.example:8802"), "GET").code, 403, "Host étranger sur l'API");
    assert_eq!(requete(port, "/", Some("192.168.1.20:8802"), "GET").code, 200, "Host IP locale");
    assert_eq!(requete(port, "/api/prevision", None, "POST").code, 501, "aucune route n'écrit");
    let t = get("/").tete;
    assert!(t.contains("X-Frame-Options: SAMEORIGIN") && t.contains("X-Content-Type-Options: nosniff"), "{t}");
    assert!(t.contains("frame-ancestors 'self'") && t.contains("Cache-Control: no-store, max-age=0"), "{t}");

    // --- Radar et tuiles
    let r = get("/api/radar");
    assert_eq!(r.code, 200);
    assert_eq!(r.json()["images"].as_array().unwrap().len(), 4);
    assert_eq!(r.json()["zoom_max"], 7);
    let r = get(&format!("/tuiles/radar/{}/7/63/46.png", *T));
    assert_eq!(r.code, 200);
    assert!(r.corps.starts_with(b"\x89PNG") && r.tete.contains("Content-Type: image/png"));
    assert_eq!(get("/tuiles/radar/1000000000/7/63/46.png").code, 404, "heure hors de la liste");
    assert_eq!(get("/tuiles/radar/1000000000/8/63/46.png").code, 404, "zoom trop fin");
    assert_eq!(get("/tuiles/osm/7/63/46.png").corps, b"PNG-OSM");

    // --- Prévision
    for q in ["", "lat=abc&lon=1", "lat=86&lon=0", "lat=0&lon=181", "lat=nan&lon=0"] {
        assert_eq!(get(&format!("/api/prevision?{q}")).code, 400, "{q}");
    }
    let r = get("/api/prevision?lat=44.8065&lon=-0.6303");
    assert_eq!(r.code, 200, "{}", String::from_utf8_lossy(&r.corps));
    let p = r.json();
    assert_eq!(p["t0"], *T);
    assert_eq!(p["arome"], true);
    assert_eq!(p["ciel"]["temperature"], 12.5);
    assert_eq!(p["commune"], "Pessac");
    assert_eq!(p["cumul_passe_mm"], 0.0, "l'averse n'est pas encore passée sur le point");
    assert_eq!(p["trajet"], Value::Null);
    assert!(p["serie"][0]["p_grele"].is_number() && p["cumul"]["mm"].is_number());
    // en mouvement
    for q in ["vitesse=45&cap=999", "vitesse=abc&cap=90"] {
        assert_eq!(get(&format!("/api/prevision?lat=44.8065&lon=-0.6303&{q}")).code, 400, "{q}");
    }
    let r = get("/api/prevision?lat=44.8065&lon=-0.6303&vitesse=45&cap=270");
    assert_eq!(r.code, 200);
    assert_eq!(r.json()["trajet"]["vitesse_kmh"], 45.0);
    // fiabilité : journal vide dans le banc
    let r = get("/api/fiabilite");
    assert_eq!(r.code, 200);
    assert_eq!(r.json()["total"], 0);
    assert_eq!(r.json()["reglage"]["n"], 0);
    assert_eq!(p["ciel"]["jour"], true);
    let serie = p["serie"].as_array().unwrap();
    assert_eq!(serie.len(), 61);
    // l'averse (bord à 30 pixels du point à l'heure T, 1 pixel par minute) arrive dans l'heure
    assert!(serie.iter().any(|m| m["p"].as_f64().unwrap() > 0.5), "{}", p["serie"]);
    assert!(serie[60]["p_cumul"].as_f64().unwrap() > 0.5);
    let v = p["mouvement"]["vitesse_kmh"].as_f64().unwrap();
    assert!((40.0..65.0).contains(&v), "1 px/min ≈ 52 km/h : {v}");
    let image = p["images"][2]["url"].as_str().unwrap().to_string();
    assert_eq!(image, format!("/prevision/{}/63/46/30.png", *T));
    let r = get(&image);
    assert_eq!(r.code, 200);
    assert!(r.corps.starts_with(b"\x89PNG"));
    let _ = std::fs::remove_dir_all(&tmp);
}

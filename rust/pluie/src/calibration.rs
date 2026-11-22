//! Réglage automatique des probabilités, d'après le journal de vérification.
//! 1. Poids du radar face à AROME, par tranche d'échéance : celui qui aurait donné le meilleur score
//!    de Brier sur les vérifications passées (au moins 100 avec AROME), sinon le poids par défaut.
//! 2. Recalibrage par classes : pour chaque tranche et chaque dixième de probabilité, l'écart entre
//!    la fréquence de pluie observée et la probabilité annoncée corrige les prévisions suivantes,
//!    d'autant plus que les vérifications sont nombreuses (n / (n + 50)). Un ensemble trop sûr de
//!    lui, ou trop prudent, est ainsi ramené à ce qui s'est vraiment passé.

use serde_json::{json, Value};

use crate::prevision::poids_radar;

const TRANCHES: usize = 3; // échéances (minutes après l'image) : jusqu'à 35, jusqu'à 65, au-delà
const CLASSES: usize = 10;
const AMORTI: f32 = 50.0;
const MIN_POIDS: usize = 100;

fn tranche(echeance: f32) -> usize {
    if echeance <= 35.0 {
        0
    } else if echeance <= 65.0 {
        1
    } else {
        2
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Calibration {
    /// Poids du radar par tranche (None : poids par défaut, trop peu de vérifications).
    pub poids: [Option<f32>; TRANCHES],
    /// Correction à ajouter au centre de chaque classe de probabilité, par tranche.
    pub ecarts: [[f32; CLASSES]; TRANCHES],
    /// Vérifications lues.
    pub n: usize,
}

/// Une vérification utile au réglage : (échéance, radar, AROME, pluie observée).
type Ligne = (f32, f32, Option<f32>, f32);

fn lignes(texte: &str) -> Vec<Ligne> {
    texte
        .lines()
        .filter_map(|l| {
            let v: Value = serde_json::from_str(l).ok()?;
            let lire = |k: &str| v.get(k).and_then(Value::as_f64).map(|x| x as f32);
            Some((lire("echeance")?, lire("p_radar")?, lire("p_arome"), lire("obs")?))
        })
        .collect()
}

impl Calibration {
    pub fn poids(&self, echeance: f32) -> f32 {
        self.poids[tranche(echeance)].unwrap_or_else(|| poids_radar(echeance))
    }

    /// La probabilité corrigée par les écarts de sa classe (interpolés entre centres de classes).
    pub fn corriger(&self, p: f32, echeance: f32) -> f32 {
        let e = &self.ecarts[tranche(echeance)];
        let x = (p * CLASSES as f32 - 0.5).clamp(0.0, (CLASSES - 1) as f32);
        let i = (x as usize).min(CLASSES - 2);
        let f = x - i as f32;
        (p + e[i] * (1.0 - f) + e[i + 1] * f).clamp(0.0, 1.0)
    }

    fn brut(&self, p_radar: f32, p_arome: Option<f32>, echeance: f32) -> f32 {
        match p_arome {
            Some(a) => {
                let w = self.poids(echeance);
                w * p_radar + (1.0 - w) * a
            }
            None => p_radar,
        }
    }

    /// Radar et AROME mélangés avec le poids réglé, puis recalibrés.
    pub fn melanger(&self, p_radar: f32, p_arome: Option<f32>, echeance: f32) -> f32 {
        self.corriger(self.brut(p_radar, p_arome, echeance), echeance)
    }

    /// Le réglage tiré des lignes du journal.
    pub fn depuis(texte: &str) -> Calibration {
        let lignes = lignes(texte);
        let mut c = Calibration { n: lignes.len(), ..Default::default() };
        for t in 0..TRANCHES {
            let avec: Vec<&Ligne> = lignes.iter().filter(|l| tranche(l.0) == t && l.2.is_some()).collect();
            if avec.len() >= MIN_POIDS {
                let brier = |w: f32| avec.iter().map(|l| (w * l.1 + (1.0 - w) * l.2.unwrap_or(0.0) - l.3).powi(2)).sum::<f32>();
                c.poids[t] = (0..=10).map(|k| k as f32 / 10.0).min_by(|a, b| brier(*a).total_cmp(&brier(*b)));
            }
        }
        let mut sommes = [[(0.0f32, 0.0f32, 0.0f32); CLASSES]; TRANCHES]; // (prévu, observé, nombre)
        for &(e, pr, pa, o) in &lignes {
            let p = c.brut(pr, pa, e);
            let s = &mut sommes[tranche(e)][((p * CLASSES as f32) as usize).min(CLASSES - 1)];
            *s = (s.0 + p, s.1 + o, s.2 + 1.0);
        }
        for t in 0..TRANCHES {
            for k in 0..CLASSES {
                let (sp, so, n) = sommes[t][k];
                c.ecarts[t][k] = (so - sp) / (n + AMORTI);
            }
        }
        c
    }

    /// Pour la page : vérifications lues, poids réglés (null : défaut), plus grand écart corrigé.
    pub fn resume(&self) -> Value {
        let max = self.ecarts.iter().flatten().fold(0.0f32, |m, e| m.max(e.abs()));
        let r = |x: f32| (x as f64 * 100.0).round() / 100.0;
        json!({"n": self.n, "poids": self.poids.iter().map(|p| p.map(r)).collect::<Vec<_>>(), "ecart_max": r(max)})
    }
}

#[cfg(test)]
mod essais {
    use super::*;

    #[test]
    fn sans_journal_rien_ne_change() {
        let c = Calibration::default();
        assert_eq!(c.melanger(0.2, Some(1.0), 80.0), 0.6, "poids par défaut");
        assert_eq!(c.melanger(0.37, None, 10.0), 0.37);
        assert_eq!(Calibration::depuis("pas du json\n"), c);
    }

    #[test]
    fn trop_sur_de_lui_ramene_a_la_realite() {
        // 200 prévisions à 80 % à 20 min, et il n'a plu que 4 fois sur 10
        let texte: String = (0..200).map(|i| format!("{{\"echeance\":20,\"p_radar\":0.8,\"p_arome\":null,\"obs\":{}}}\n", (i % 10 < 4) as u8)).collect();
        let c = Calibration::depuis(&texte);
        assert_eq!(c.n, 200);
        // écart de la classe 0,8 : 200 × (0,4 − 0,8) / 250 = −0,32
        assert!((c.ecarts[0][8] + 0.32).abs() < 1e-4, "{:?}", c.ecarts[0]);
        assert!((c.corriger(0.85, 20.0) - 0.53).abs() < 1e-3);
        assert_eq!(c.corriger(0.85, 50.0), 0.85, "autre tranche : rien de connu");
        assert_eq!(c.poids, [None; 3], "sans AROME, pas de poids réglé");
    }

    #[test]
    fn poids_choisi_sur_le_score() {
        // à 50 min, AROME a toujours raison et le radar dit toujours 50 % : tout le poids à AROME
        let texte: String = (0..150).map(|i| format!("{{\"echeance\":50,\"p_radar\":0.5,\"p_arome\":{o},\"obs\":{o}}}\n", o = i % 2)).collect();
        let c = Calibration::depuis(&texte);
        assert_eq!(c.poids, [None, Some(0.0), None]);
        assert_eq!(c.poids(50.0), 0.0);
        assert_eq!(c.poids(80.0), poids_radar(80.0));
        assert_eq!(c.resume()["poids"][1], 0.0);
    }
}

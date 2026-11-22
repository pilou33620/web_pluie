//! web_pluie : carte radar des précipitations et probabilité de pluie sur sa position, de 5 min à
//! 1 h. radar.rs décode les tuiles RainViewer ; mouvement.rs (flux optique, advection), cellules.rs
//! (suivi des averses) et prevision.rs (ensemble, AROME) prévoient ; scene.rs assemble le tout pour
//! un lieu ; journal.rs vérifie les prévisions après coup et calibration.rs en tire le réglage ; serveur.rs et sources.rs servent et relaient.
pub mod calibration;
pub mod cellules;
pub mod journal;
pub mod mouvement;
pub mod prevision;
pub mod radar;
pub mod scene;
pub mod serveur;
pub mod sources;

// web_pluie : le fond d'écran qui suit la météo, repris du fond « Paysage » de tooth_dentaire.
// Ciel selon le moment de la journée et le temps, soleil ou lune, nuages qui dérivent, collines et
// une maison (fenêtres allumées le soir et sous la pluie) ; pluie, neige ou étoiles animées sur un
// canvas, éclairs par temps d'orage (CSS). Le temps vient de la prévision : radar sur le point pour
// la pluie et la neige, AROME pour la nébulosité, le lever et le coucher du soleil.
'use strict';
(function (racine) {
  const MOMENTS = ['aube', 'jour', 'crepuscule', 'nuit'];
  const METEOS = ['clair', 'voile', 'nuageux', 'couvert', 'brouillard', 'pluie', 'neige', 'orage'];
  const MAUVAIS = ['couvert', 'brouillard', 'pluie', 'neige', 'orage'];
  const LIBELLES = { clair: 'Ciel dégagé', voile: 'Voile nuageux', nuageux: 'Nuageux', couvert: 'Couvert',
    brouillard: 'Brouillard', pluie: 'Pluie', neige: 'Neige', orage: 'Orage' };

  // ---------- Palettes ----------
  const CIEL = {
    aube: ['#4d5f96', '#c98ba8', '#f6b38f'],
    jour: ['#3f95d1', '#7fc2e6', '#c9eaf2'],
    crepuscule: ['#2f2766', '#a0558a', '#f2955f'],
    nuit: ['#060a22', '#141b44', '#283062']
  };
  const CIEL_METEO = {
    couvert: { jour: ['#6f7f8d', '#98a6b1', '#c3ccd3'], nuit: ['#10151d', '#1d252f', '#2f3842'] },
    brouillard: { jour: ['#8e99a3', '#b4bcc3', '#d8dde0'], nuit: ['#141a21', '#252d36', '#3b434c'] },
    pluie: { jour: ['#4b5864', '#717e88', '#9ca7af'], nuit: ['#0b1117', '#18212b', '#29323c'] },
    orage: { jour: ['#22263a', '#3d4257', '#5e6277'], nuit: ['#05070d', '#11131d', '#1f2230'] },
    neige: { jour: ['#8597ab', '#b4c2cf', '#e1e8ee'], nuit: ['#141e2c', '#273447', '#3e4c61'] }
  };
  const COLLINES = {
    aube: ['#7a6f93', '#4d5674', '#323a56'],
    jour: ['#6aa89a', '#3f8571', '#2d6655'],
    crepuscule: ['#7a4f73', '#4c3654', '#2f2340'],
    nuit: ['#1b2740', '#131c30', '#0c1322']
  };
  const COLLINES_GRIS = { jour: ['#7f948f', '#5a7069', '#435650'], nuit: ['#1c2430', '#141b25', '#0d131b'] };
  const COLLINES_ORAGE = ['#4e5a5c', '#384345', '#283133'];
  const COLLINES_NEIGE = { jour: ['#b4c2cf', '#c3cfda', '#d0dae3'], nuit: ['#8796ab', '#a1afc1', '#b7c3d2'] };
  const ASTRE = { aube: [230, 430], jour: [1010, 120], crepuscule: [1150, 395], nuit: [1000, 110] };

  // ---------- Choix du décor ----------

  /** Moment de la journée à l'heure t (secondes), d'après le lever et le coucher du jour ;
   *  sans eux, d'après l'heure locale. */
  function moment(t, lever, coucher) {
    if (lever == null || coucher == null) {
      const h = new Date(t * 1000).getHours();
      return h < 6 || h >= 21 ? 'nuit' : h < 8 ? 'aube' : h >= 19 ? 'crepuscule' : 'jour';
    }
    const marge = 2700; // 45 min autour du lever et du coucher
    if (t < lever - marge || t > coucher + marge) return 'nuit';
    if (t < lever + marge) return 'aube';
    if (t > coucher - marge) return 'crepuscule';
    return 'jour';
  }

  /** Le temps sur le point, d'après une prévision de /api/prevision (null : beau temps). */
  function meteoDe(p) {
    if (!p) return { meteo: 'clair', intensite: 'moyenne' };
    const c = p.ciel || {}, dbz = p.point ? p.point.dbz : null;
    if (dbz != null && dbz >= 12) {
      const mm = (p.serie[0] && p.serie[0].mm_h) || 0.5;
      const intensite = mm < 1 ? 'faible' : mm < 4 ? 'moyenne' : 'forte';
      if (p.point.neige) return { meteo: 'neige', intensite };
      if (dbz >= 45 || [95, 96, 99].includes(c.code)) return { meteo: 'orage', intensite: 'forte' };
      return { meteo: 'pluie', intensite };
    }
    if ([45, 48].includes(c.code)) return { meteo: 'brouillard', intensite: 'moyenne' };
    let meteo = c.nuages != null ? (c.nuages < 20 ? 'clair' : c.nuages < 50 ? 'voile' : c.nuages < 85 ? 'nuageux' : 'couvert')
      : c.code != null ? (['clair', 'voile', 'nuageux'][c.code] || 'couvert') : 'clair';
    // la pluie approche (probable dans le quart d'heure) : le ciel se charge
    if (meteo !== 'couvert' && p.serie.slice(0, 16).some(m => m.p >= 0.5)) meteo = 'couvert';
    return { meteo, intensite: 'moyenne' };
  }

  /** Tout ce qui varie dans le décor. */
  function decor(mo, meteo, nuit) {
    const m = MOMENTS.includes(mo) ? mo : 'jour';
    const temps = METEOS.includes(meteo) ? meteo : 'clair';
    const estNuit = typeof nuit === 'boolean' ? nuit : m === 'nuit';
    const jn = estNuit ? 'nuit' : 'jour';
    const mauvais = MAUVAIS.includes(temps);
    let collines = COLLINES[estNuit ? 'nuit' : m];
    if (temps === 'neige') collines = COLLINES_NEIGE[jn];
    else if (temps === 'orage' && !estNuit) collines = COLLINES_ORAGE;
    else if (mauvais) collines = COLLINES_GRIS[jn];
    return {
      moment: m, meteo: temps, nuit: estNuit,
      ciel: mauvais ? CIEL_METEO[temps][jn] : CIEL[estNuit ? 'nuit' : m],
      collines,
      astre: mauvais ? null : estNuit ? 'lune' : 'soleil',
      etoiles: estNuit && (temps === 'clair' || temps === 'voile'),
      fenetres: m !== 'jour' || estNuit || temps === 'pluie' || temps === 'orage',
      toitEnneige: temps === 'neige',
      brouillard: temps === 'brouillard'
    };
  }

  // ---------- Dessin (SVG en chaîne : uniquement des constantes et des nombres) ----------
  function nuage(x, y, echelle, couleur, duree, decalage) {
    return `<g class="p-derive" style="animation-duration:${duree}s;animation-delay:-${decalage}s">`
      + `<g transform="translate(${x} ${y}) scale(${echelle})" fill="${couleur}">`
      + '<ellipse cx="0" cy="18" rx="70" ry="24"/><circle cx="-28" cy="6" r="28"/>'
      + '<circle cx="10" cy="-4" r="38"/><circle cx="44" cy="10" r="26"/></g></g>';
  }

  function nuages(d) {
    const blanc = d.nuit ? 'rgba(170,180,205,.35)' : 'rgba(255,255,255,.88)';
    const gris = d.nuit ? 'rgba(70,80,96,.75)' : 'rgba(214,221,228,.92)';
    const neige = d.nuit ? 'rgba(120,135,160,.6)' : 'rgba(236,241,246,.95)';
    const sombre = d.nuit ? 'rgba(30,34,46,.9)' : 'rgba(90,96,114,.92)';
    const couvrant = [[120, 80, 1.6, 200], [520, 60, 1.8, 230], [900, 110, 1.5, 210], [330, 190, 1.3, 180], [760, 220, 1.2, 190]];
    const plan = {
      voile: [[[300, 150, 0.8, 160]], blanc],
      nuageux: [[[200, 140, 1.1, 150], [760, 90, 0.9, 190], [520, 230, 0.7, 170]], blanc],
      couvert: [couvrant, gris],
      brouillard: [couvrant, gris],
      pluie: [couvrant, gris],
      neige: [couvrant, neige],
      orage: [couvrant.map(([x, y, s, t]) => [x, y + 10, s + 0.1, t + 20]), sombre]
    }[d.meteo];
    if (!plan) return '';
    return plan[0].map(([x, y, s, duree], i) => nuage(x, y, s, plan[1], duree, i * 29)).join('');
  }

  function etoiles() {
    let points = '';
    for (let i = 0; i < 70; i++) {
      const x = (i * 197.3) % 1280, y = (i * 83.7) % 380;
      points += `<circle cx="${x.toFixed(0)}" cy="${y.toFixed(0)}" r="${(0.6 + ((i * 7) % 10) / 10).toFixed(1)}" opacity="${(0.35 + ((i * 13) % 10) / 16).toFixed(2)}"/>`;
    }
    return `<g class="p-etoiles" fill="#fff">${points}</g>`;
  }

  function astre(d) {
    if (!d.astre) return '';
    const [x, y] = ASTRE[d.nuit ? 'nuit' : d.moment];
    if (d.astre === 'lune') {
      return `<g transform="translate(${x} ${y})"><circle r="70" fill="rgba(220,230,255,.08)"/><circle r="38" fill="#f4f1e2"/>`
        + `<circle cx="16" cy="-10" r="34" fill="${d.ciel[0]}"/></g>`;
    }
    const c = d.moment === 'jour' ? '#fff6d4' : '#ffd59a';
    return `<g class="p-soleil" transform="translate(${x} ${y})"><circle r="150" fill="${c}" opacity=".18"/>`
      + `<circle r="85" fill="${c}" opacity=".28"/><circle r="46" fill="${c}"/></g>`;
  }

  function maison(d) {
    const fenetre = d.fenetres ? '#ffd27a' : (MAUVAIS.includes(d.meteo) ? '#c7d3db' : '#a9d6ea');
    const mur = d.nuit ? '#2c3549' : d.moment === 'crepuscule' ? '#e9d9cf' : '#f6efe4';
    const toit = d.toitEnneige ? (d.nuit ? '#c9d3df' : '#ffffff') : d.nuit ? '#3b3346' : '#c06a47';
    const porte = d.nuit ? '#1b2232' : '#8a5a3c';
    const lueur = d.fenetres ? ' class="p-allumee"' : '';
    return '<g transform="translate(960 392) scale(.42)">'
      + `<rect x="150" y="-60" width="26" height="50" fill="${d.nuit ? '#3a3040' : '#9c5a3e'}"/>`
      + `<rect x="0" y="0" width="230" height="120" fill="${mur}"/>`
      + `<path d="M-16 6 L115 -70 L246 6 Z" fill="${toit}"/>`
      + `<rect x="92" y="48" width="46" height="72" rx="4" fill="${porte}"/>`
      + `<rect${lueur} x="22" y="34" width="44" height="36" rx="4" fill="${fenetre}"/>`
      + `<rect${lueur} x="164" y="34" width="44" height="36" rx="4" fill="${fenetre}"/></g>`;
  }

  function brume(d) {
    if (!d.brouillard) return '';
    const c = d.nuit ? 'rgba(110,120,135,.5)' : 'rgba(240,244,247,.72)';
    return `<g><rect x="-100" y="380" width="1480" height="160" fill="${c}" filter="url(#p-flou)"/>`
      + `<rect x="-100" y="560" width="1480" height="160" fill="${c}" filter="url(#p-flou)"/></g>`;
  }

  /** Le décor complet (SVG). Le sol reste en bas de l'écran, le ciel déborde vers le haut. */
  function dessiner(d) {
    const [c0, c1, c2] = d.collines;
    return '<svg viewBox="0 0 1280 720" preserveAspectRatio="xMidYMax slice" aria-hidden="true" focusable="false">'
      + '<defs><linearGradient id="p-ciel" x1="0" y1="0" x2="0" y2="1">'
      + `<stop offset="0" stop-color="${d.ciel[0]}"/><stop offset=".55" stop-color="${d.ciel[1]}"/><stop offset="1" stop-color="${d.ciel[2]}"/></linearGradient>`
      + '<filter id="p-flou" x="-10%" y="-50%" width="120%" height="200%"><feGaussianBlur stdDeviation="22"/></filter></defs>'
      + '<rect x="-200" y="-1200" width="1680" height="1920" fill="url(#p-ciel)"/>'
      + (d.etoiles ? etoiles() : '') + astre(d) + nuages(d)
      + `<path d="M0 470 C 180 400, 330 430, 480 455 C 650 485, 780 395, 960 420 C 1100 440, 1200 410, 1280 425 L1280 720 L0 720 Z" fill="${c0}"/>`
      + `<path d="M0 560 C 220 500, 380 545, 560 560 C 760 575, 900 520, 1080 540 C 1180 552, 1240 545, 1280 548 L1280 720 L0 720 Z" fill="${c1}"/>`
      + maison(d)
      + `<path d="M0 640 C 260 610, 520 650, 780 640 C 1000 632, 1160 650, 1280 645 L1280 720 L0 720 Z" fill="${c2}"/>`
      + brume(d) + '</svg>';
  }

  // ---------- Particules (pluie, neige, étoiles) ----------
  function particulesPour(meteo, intensite, nuit) {
    const selon = (f, m, F) => (intensite === 'faible' ? f : intensite === 'forte' ? F : m);
    switch (meteo) {
      case 'pluie': return { type: 'pluie', nombre: selon(70, 140, 240) };
      case 'orage': return { type: 'pluie', nombre: 240 };
      case 'neige': return { type: 'neige', nombre: selon(60, 110, 180) };
      case 'clair': case 'voile': return nuit ? { type: 'etoiles', nombre: meteo === 'clair' ? 140 : 70 } : null;
      default: return null;
    }
  }

  let animation = null;
  function arreter() {
    if (!animation) return;
    cancelAnimationFrame(animation.raf);
    window.removeEventListener('resize', animation.taille);
    animation.ctx.clearRect(0, 0, animation.canvas.width, animation.canvas.height);
    animation = null;
  }

  function animer(canvas, spec) {
    const cle = spec ? `${spec.type}-${spec.nombre}` : '';
    if (animation && animation.cle === cle) return;
    arreter();
    if (!spec || !canvas || typeof requestAnimationFrame !== 'function') return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const a = { cle, canvas, ctx, raf: 0, dernier: 0, parts: [], l: 0, h: 0, t: 0 };
    const hasard = (min, max) => min + Math.random() * (max - min);
    function creer(p, depart) {
      if (spec.type === 'pluie') {
        p.x = hasard(-0.1, 1.1) * a.l; p.y = depart ? hasard(0, a.h) : hasard(-60, -10);
        p.v = hasard(0.9, 1.4) * a.h * 0.9; p.long = hasard(10, 22); p.a = hasard(0.18, 0.45);
      } else if (spec.type === 'neige') {
        p.x = hasard(0, a.l); p.y = depart ? hasard(0, a.h) : hasard(-20, -4);
        p.v = hasard(25, 70); p.r = hasard(1, 3.2); p.ph = hasard(0, Math.PI * 2); p.a = hasard(0.5, 0.95);
      } else {
        p.x = hasard(0, a.l); p.y = hasard(0, a.h * 0.62); p.r = hasard(0.5, 1.4); p.ph = hasard(0, Math.PI * 2); p.vit = hasard(0.6, 1.8);
      }
      return p;
    }
    a.taille = () => {
      const dpr = Math.min(1.5, window.devicePixelRatio || 1);
      a.l = window.innerWidth; a.h = window.innerHeight;
      canvas.width = Math.round(a.l * dpr); canvas.height = Math.round(a.h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      a.parts = Array.from({ length: spec.nombre }, () => creer({}, true));
    };
    a.taille();
    window.addEventListener('resize', a.taille);
    function dessinerParts(dt) {
      ctx.clearRect(0, 0, a.l, a.h);
      a.t += dt;
      for (const p of a.parts) {
        if (spec.type === 'pluie') {
          p.y += p.v * dt; p.x += p.v * 0.12 * dt;
          if (p.y - p.long > a.h) creer(p, false);
          ctx.strokeStyle = `rgba(220,235,255,${p.a})`; ctx.lineWidth = 1.1; ctx.lineCap = 'round';
          ctx.beginPath(); ctx.moveTo(p.x, p.y); ctx.lineTo(p.x - p.long * 0.12, p.y - p.long); ctx.stroke();
        } else if (spec.type === 'neige') {
          p.y += p.v * dt; p.x += Math.sin(a.t * 0.8 + p.ph) * 18 * dt;
          if (p.y - p.r > a.h) creer(p, false);
          ctx.fillStyle = `rgba(255,255,255,${p.a})`;
          ctx.beginPath(); ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2); ctx.fill();
        } else {
          const s = 0.55 + 0.45 * Math.sin(a.t * p.vit + p.ph);
          ctx.fillStyle = `rgba(255,255,255,${(0.25 + 0.6 * s).toFixed(3)})`;
          ctx.beginPath(); ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2); ctx.fill();
        }
      }
    }
    const pas = spec.type === 'etoiles' ? 1000 / 12 : 1000 / 30; // images par seconde : le Pi et le téléphone respirent
    function boucle(t) {
      a.raf = requestAnimationFrame(boucle);
      if (document.hidden) { a.dernier = t; return; }
      if (t - a.dernier < pas) return;
      const dt = a.dernier ? Math.min(0.1, (t - a.dernier) / 1000) : 0;
      a.dernier = t;
      dessinerParts(dt);
    }
    animation = a;
    let fixe = false;
    try { fixe = window.matchMedia('(prefers-reduced-motion: reduce)').matches; } catch { /* vieux navigateur */ }
    if (fixe) dessinerParts(0); else a.raf = requestAnimationFrame(boucle);
  }

  // ---------- Application ----------
  let dernier = '';
  /** Met le fond au temps de la prévision p (ou au seul moment de la journée sans elle). */
  function appliquer(p, maintenant) {
    const c = (p && p.ciel) || {};
    const mo = moment(maintenant, c.lever, c.coucher);
    const { meteo, intensite } = meteoDe(p);
    const d = decor(mo, meteo, typeof c.jour === 'boolean' ? !c.jour : mo === 'nuit');
    const html = document.documentElement;
    html.dataset.meteo = d.meteo;
    html.dataset.nuit = d.nuit ? 'oui' : 'non';
    const cle = `${d.moment}|${d.meteo}|${d.nuit}`;
    if (cle !== dernier) {
      dernier = cle;
      document.getElementById('paysage').innerHTML = dessiner(d);
      const meta = document.querySelector('meta[name=theme-color]');
      if (meta) meta.content = d.ciel[0];
    }
    animer(document.getElementById('particules'), particulesPour(d.meteo, intensite, d.nuit));
    return d;
  }

  const P = { MOMENTS, METEOS, LIBELLES, moment, meteoDe, decor, dessiner, particulesPour, appliquer };
  if (typeof module !== 'undefined' && module.exports) module.exports = P;
  else racine.Paysage = P;
})(this);

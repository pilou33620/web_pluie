// web_pluie : la page. Position GPS → /api/prevision toutes les 2 min → phrase, courbes, tableau,
// carte (radar passé, pluie prévue, flèches de déplacement, averses suivies), notifications.
'use strict';
(() => {
  const $ = s => document.querySelector(s);
  const NS = 'http://www.w3.org/2000/svg';
  const C1 = '#2a78d6', C2 = '#eb6834', FOND = '#f2f2f6'; // séries (palette validée) et fond des cartes
  const RAFRAICHIR = 120000;
  const K0 = 5, K1 = 60; // fenêtre affichée : de +5 min à +1 h
  const MARGES = { g: 40, d: 42 }; // communes aux deux graphiques : les colonnes tombent sous la courbe

  const REGLAGES = 'web_pluie.reglages';
  const R = Object.assign({ fleches: false, cellules: true, notifs: false, seuil: 0.5 }, (() => {
    try { return JSON.parse(localStorage.getItem(REGLAGES)) || {}; } catch { return {}; }
  })());
  const garder = () => { try { localStorage.setItem(REGLAGES, JSON.stringify(R)); } catch { /* navigation privée */ } };

  const etat = { pos: null, veille: null, prev: null, radar: [], images: [], cadre: -1, couche: null, minuteur: null, lecture: null, alerte: false, vue: false };

  // ---------- Outils ----------
  const pct = p => `${Math.round(p * 100)} %`;
  const hm = t => new Date(t * 1000).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
  const DIRS = ['le nord', 'le nord-est', "l'est", 'le sud-est', 'le sud', 'le sud-ouest', "l'ouest", 'le nord-ouest'];
  const versDir = d => DIRS[Math.round(d / 45) % 8];
  const mmTexte = mm => `${mm < 10 ? mm.toFixed(1).replace('.', ',') : Math.round(mm)} mm/h`;
  const force = mm => (mm < 1 ? 'faible' : mm < 4 ? 'modérée' : mm < 10 ? 'forte' : 'très forte');
  const km = (a, b) => {
    const r = Math.PI / 180, x = (b.lon - a.lon) * r * Math.cos((a.lat + b.lat) / 2 * r), y = (b.lat - a.lat) * r;
    return 6371 * Math.hypot(x, y);
  };

  async function api(url) {
    const r = await fetch(url, { cache: 'no-store' });
    const j = await r.json().catch(() => ({}));
    if (r.status === 401 || r.status === 403) throw new Error('appareil non autorisé : rechargez la page pour entrer le code');
    if (!r.ok) throw new Error(j.detail || `HTTP ${r.status}`);
    return j;
  }

  function el(nom, attrs, parent) {
    const e = document.createElementNS(NS, nom);
    for (const k in attrs) e.setAttribute(k, attrs[k]);
    if (parent) parent.appendChild(e);
    return e;
  }
  function texte(parent, x, y, t, cls, ancre) {
    const e = el('text', { x, y, class: cls, 'text-anchor': ancre || 'start' }, parent);
    e.textContent = t;
    return e;
  }

  // ---------- Carte ----------
  const carte = L.map('carte', { center: [46.6, 2.4], zoom: 6, maxZoom: 12, zoomControl: false });
  L.tileLayer('/tuiles/osm/{z}/{x}/{y}.png', { maxZoom: 12,
    attribution: '<a href="https://www.openstreetmap.org/copyright">© OpenStreetMap</a> · Radar <a href="https://www.rainviewer.com">RainViewer</a>' }).addTo(carte);
  L.control.zoom({ position: 'topright' }).addTo(carte);
  const groupes = { point: L.layerGroup().addTo(carte), fleches: L.layerGroup(), cellules: L.layerGroup() };

  function cadres() {
    const passe = etat.radar.map(t => ({ t, futur: false }));
    const p = etat.prev, fin = Date.now() / 1000 + 3600;
    const futur = p ? p.images.filter(i => i.t <= fin + 300).map(i => ({ t: i.t, futur: true, url: i.url })) : [];
    return passe.concat(futur);
  }

  function majCurseur() {
    const avant = etat.images[etat.cadre];
    etat.images = cadres();
    $('#curseur').max = Math.max(0, etat.images.length - 1);
    // on garde l'heure regardée si elle existe encore, sinon la dernière image observée
    let k = avant ? etat.images.findIndex(i => i.t === avant.t && i.futur === avant.futur) : -1;
    if (k < 0) k = etat.images.filter(i => !i.futur).length - 1;
    afficherCadre(Math.max(0, k));
  }

  function afficherCadre(k) {
    const im = etat.images[k];
    if (!im) return;
    const meme = k === etat.cadre && etat.couche && etat.couche._cle === `${im.t}${im.url || ''}`;
    etat.cadre = k;
    $('#curseur').value = k;
    $('#curseur-t').textContent = `${hm(im.t)}${im.futur ? ' prévu' : ''}`;
    if (meme) return;
    let couche;
    if (im.futur) {
      const d = etat.prev.domaine;
      couche = L.imageOverlay(im.url, [[d.sud, d.ouest], [d.nord, d.est]], { opacity: 0.75, zIndex: 2 });
    } else {
      couche = L.tileLayer(`/tuiles/radar/${im.t}/{z}/{x}/{y}.png`, { opacity: 0.75, zIndex: 2, maxNativeZoom: 7, maxZoom: 12 });
    }
    couche._cle = `${im.t}${im.url || ''}`;
    const ancienne = etat.couche;
    etat.couche = couche;
    couche.addTo(carte);
    if (ancienne) {
      // l'ancienne image reste le temps que la nouvelle arrive : pas de clignotement
      const retirer = () => carte.removeLayer(ancienne);
      couche.once('load', retirer);
      setTimeout(retirer, 1500);
    }
  }

  $('#curseur').addEventListener('input', e => { arreter(); afficherCadre(+e.target.value); });
  function arreter() {
    clearInterval(etat.lecture);
    etat.lecture = null;
    $('#play').textContent = '▶';
    $('#play').setAttribute('aria-label', 'Lecture');
  }
  $('#play').addEventListener('click', () => {
    if (etat.lecture) return arreter();
    $('#play').textContent = '❚❚';
    $('#play').setAttribute('aria-label', 'Pause');
    etat.lecture = setInterval(() => afficherCadre((etat.cadre + 1) % Math.max(1, etat.images.length)), 800);
  });

  function infobulle(t) {
    const s = document.createElement('span');
    s.textContent = t;
    return s;
  }

  function dessinerCarte(p) {
    groupes.point.clearLayers();
    if (etat.pos) {
      L.circle([etat.pos.lat, etat.pos.lon], { radius: Math.min(etat.pos.precision || 0, 3000), color: C1, weight: 1, fillOpacity: 0.08, interactive: false }).addTo(groupes.point);
      L.circleMarker([etat.pos.lat, etat.pos.lon], { radius: 7, color: '#fff', weight: 2, fillColor: C1, fillOpacity: 1 })
        .bindTooltip(infobulle('Votre position')).addTo(groupes.point);
    }
    groupes.fleches.clearLayers();
    groupes.cellules.clearLayers();
    if (!p) return;
    for (const v of p.vecteurs) {
      if (Math.hypot(v.u, v.v) < 3) continue;
      const [la, lo] = v.position, h = 0.25; // la flèche montre 15 min de déplacement
      const fin = [la + v.v * h / 111.2, lo + v.u * h / (111.2 * Math.cos(la * Math.PI / 180))];
      L.polyline([[la, lo], fin], { color: '#1d3b5e', weight: 1.5, opacity: 0.75, interactive: false }).addTo(groupes.fleches);
      L.circleMarker(fin, { radius: 2.5, stroke: false, fillColor: '#1d3b5e', fillOpacity: 0.9, interactive: false }).addTo(groupes.fleches);
    }
    for (const c of p.cellules) {
      const t = `Averse forte : ${c.dbz_max} dBZ, ${c.vitesse_kmh} km/h` + (c.arrivee_min != null ? `, sur vous dans ~${c.arrivee_min} min` : '');
      L.circle(c.position, { radius: c.rayon_km * 1000, color: '#a80000', weight: 1.5, fillOpacity: 0.1 }).bindTooltip(infobulle(t)).addTo(groupes.cellules);
      if (c.passe.length > 1) L.polyline(c.passe, { color: '#a80000', weight: 1.5, opacity: 0.5, interactive: false }).addTo(groupes.cellules);
      L.polyline([c.position, ...c.prevue], { color: '#a80000', weight: 1.5, dashArray: '4 5', interactive: false }).addTo(groupes.cellules);
    }
  }

  function majCouches() {
    for (const [nom, on] of [['fleches', R.fleches], ['cellules', R.cellules]]) {
      if (on) groupes[nom].addTo(carte); else carte.removeLayer(groupes[nom]);
    }
  }

  // ---------- Position ----------
  function sansGps(msg) {
    $('#sans-gps').hidden = false;
    $('#sans-gps-texte').textContent = msg;
    $('#phrase').textContent = 'Position inconnue';
    $('#detail').textContent = 'La carte radar reste affichée ; la probabilité sur vous attend votre position.';
    for (const s of ['#fig-p', '#fig-i', '#tableau-bloc']) $(s).hidden = true;
    majEtat();
  }

  function suivreGps() {
    if (!window.isSecureContext) {
      return sansGps("La position GPS demande une connexion sécurisée (https) : installez le certificat du poste (page d'appairage), ou ouvrez la page sur le poste lui-même.");
    }
    if (!navigator.geolocation) return sansGps('Ce navigateur ne donne pas la position.');
    if (etat.veille !== null) navigator.geolocation.clearWatch(etat.veille);
    etat.veille = navigator.geolocation.watchPosition(p => {
      const nouv = { lat: p.coords.latitude, lon: p.coords.longitude, precision: p.coords.accuracy };
      const bouge = !etat.pos || km(etat.pos, nouv) > 0.5;
      etat.pos = nouv;
      $('#sans-gps').hidden = true;
      if (!etat.vue) {
        etat.vue = true;
        carte.setView([nouv.lat, nouv.lon], 8);
      }
      dessinerCarte(etat.prev);
      if (bouge) charger();
    }, e => {
      if (!etat.pos) sansGps(e.code === 1 ? 'Position refusée : autorisez la localisation pour ce site dans le navigateur.' : "Position indisponible pour l'instant.");
    }, { enableHighAccuracy: true, maximumAge: 60000, timeout: 30000 });
  }
  $('#b-gps').addEventListener('click', suivreGps);

  // ---------- Chargement ----------
  async function charger() {
    clearTimeout(etat.minuteur);
    etat.minuteur = setTimeout(charger, RAFRAICHIR);
    try {
      etat.radar = (await api('/api/radar')).images;
    } catch (e) {
      $('#etat').textContent = `Radar indisponible : ${e.message}`;
    }
    if (etat.pos) {
      try {
        etat.prev = await api(`/api/prevision?lat=${etat.pos.lat.toFixed(4)}&lon=${etat.pos.lon.toFixed(4)}`);
        afficherPrevision(etat.prev);
      } catch (e) {
        $('#phrase').textContent = 'Prévision indisponible';
        $('#detail').textContent = e.message;
      }
    }
    majCurseur();
    majEtat();
  }

  function majEtat() {
    const t = etat.prev ? etat.prev.t0 : etat.radar[etat.radar.length - 1];
    if (!t) return;
    const age = Math.round((Date.now() / 1000 - t) / 60);
    $('#etat').textContent = `Radar de ${hm(t)} (il y a ${age} min)${age > 30 ? ' : ancien' : ''}${etat.pos ? '' : ' · sans position'}`;
  }

  // ---------- Prévision ----------
  function resume(p) {
    const s = p.serie, quoi = p.point.neige ? 'Neige' : 'Pluie';
    if (!s.length) return ['Radar trop ancien pour prévoir', ''];
    const fin = s[s.length - 1], duree = s.length > 60 ? '1 h' : `${s.length - 1} min`;
    const ici = p.point.dbz != null && p.point.dbz >= 12;
    if (ici || s[0].p >= 0.5) {
      const k = s.findIndex((m, i) => i >= K0 && m.p < 0.3);
      const mm = s[0].mm_h || s[Math.min(5, s.length - 1)].mm_h;
      return [`${p.point.neige ? 'Il neige' : 'Il pleut'} sur vous${mm ? ` (${force(mm)})` : ''}`,
        k < 0 ? `et cela devrait durer au-delà de ${duree}` : `fin probable dans ~${k} min`];
    }
    const k = s.findIndex((m, i) => i >= K0 && m.p >= 0.5);
    if (k >= 0) return [`${quoi} probable dans ~${k} min (${pct(s[k].p)})`, s[k].mm_h ? `intensité ${force(s[k].mm_h)}, ~${mmTexte(s[k].mm_h)}` : ''];
    if (fin.p_cumul >= 0.15) return [`Risque de ${quoi.toLowerCase()} d'ici ${duree} : ${pct(fin.p_cumul)}`, ''];
    return [`Pas de ${quoi.toLowerCase()} attendue d'ici ${duree}`, `risque ${pct(fin.p_cumul)}`];
  }

  function afficherPrevision(p) {
    const [phrase, suite] = resume(p);
    $('#phrase').textContent = phrase;
    const mv = p.mouvement;
    const deplacement = mv.blocs > 0 ? `pluie qui va vers ${versDir(mv.direction)} à ${mv.vitesse_kmh} km/h` : 'aucune pluie à suivre autour de vous';
    $('#detail').textContent = [suite, deplacement, p.arome ? 'radar + AROME' : 'radar seul'].filter(Boolean).join(' · ');
    const avecSerie = p.serie.length > K0;
    $('#fig-p').hidden = $('#tableau-bloc').hidden = !avecSerie;
    if (avecSerie) {
      dessinerCourbe(p.serie);
      dessinerBarres(p.serie);
      remplirTableau(p.serie);
    } else {
      $('#fig-i').hidden = true;
    }
    dessinerCarte(p);
    verifierAlerte(p);
  }

  // ---------- Graphiques ----------
  const geo = {}; // échelles du dernier dessin, pour le survol
  function echelleX(W) {
    return k => MARGES.g + (k - K0) / (K1 - K0) * (W - MARGES.g - MARGES.d);
  }
  const chemin = pts => pts.map((q, i) => `${i ? 'L' : 'M'}${q[0].toFixed(1)},${q[1].toFixed(1)}`).join('');
  const minutes = s => { const k = []; for (let i = K0; i <= Math.min(K1, s.length - 1); i++) k.push(i); return k; };

  function dessinerCourbe(s) {
    const svg = $('#courbe');
    svg.replaceChildren();
    const W = svg.clientWidth || 340, H = svg.clientHeight || 190, haut = 8, bas = 22;
    svg.setAttribute('viewBox', `0 0 ${W} ${H}`);
    const x = echelleX(W), y = p => haut + (1 - p) * (H - haut - bas);
    for (const p of [0, 0.5, 1]) {
      el('line', { x1: MARGES.g, x2: W - MARGES.d, y1: y(p), y2: y(p), class: 'grille' }, svg);
      texte(svg, MARGES.g - 6, y(p) + 4, pct(p), 'axe', 'end');
    }
    for (const k of [5, 15, 30, 45, 60]) texte(svg, x(k), H - 5, k === 5 ? '+5 min' : `+${k}`, 'axe', k === 5 ? 'start' : k === 60 ? 'end' : 'middle');
    const ks = minutes(s);
    const pp = ks.map(k => [x(k), y(s[k].p)]), pc = ks.map(k => [x(k), y(s[k].p_cumul)]);
    const [xa, xb] = [pp[0][0], pp[pp.length - 1][0]];
    el('path', { d: `${chemin(pp)}L${xb.toFixed(1)},${y(0)}L${xa.toFixed(1)},${y(0)}Z`, fill: C1, 'fill-opacity': 0.1 }, svg);
    for (const [pts, c] of [[pc, C2], [pp, C1]]) {
      el('path', { d: chemin(pts), fill: 'none', stroke: c, 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }, svg);
    }
    // valeurs en bout de courbe, écartées si elles se chevauchent
    const kf = ks[ks.length - 1];
    let [ya, yb] = [y(s[kf].p), y(s[kf].p_cumul)];
    if (Math.abs(ya - yb) < 14) { const m = (ya + yb) / 2, e = ya <= yb ? -7 : 7; ya = m + e; yb = m - e; }
    for (const [p, c, yt] of [[s[kf].p_cumul, C2, yb], [s[kf].p, C1, ya]]) {
      el('circle', { cx: xb, cy: y(p), r: 4, fill: c, stroke: FOND, 'stroke-width': 2 }, svg);
      texte(svg, xb + 8, yt + 4, pct(p), 'valeur');
    }
    const curseur = el('g', { visibility: 'hidden' }, svg);
    el('line', { y1: haut, y2: H - bas, stroke: '#8e8e93', 'stroke-width': 1 }, curseur);
    for (const c of [C2, C1]) el('circle', { r: 4, fill: c, stroke: FOND, 'stroke-width': 2 }, curseur);
    Object.assign(geo, { s, x, y, W, curseur, ks });
    brancherSurvol(svg);
  }

  function dessinerBarres(s) {
    const ks = minutes(s), mm = ks.map(k => s[k].mm_h), max = Math.max(...mm);
    $('#fig-i').hidden = !(max > 0);
    if (!(max > 0)) return;
    const svg = $('#barres');
    svg.replaceChildren();
    const W = svg.clientWidth || 340, H = svg.clientHeight || 84, haut = 8, bas = 16;
    svg.setAttribute('viewBox', `0 0 ${W} ${H}`);
    const plafond = [1, 2, 5, 10, 20, 50, 100, 200].find(v => v >= max) || max;
    const x = echelleX(W), y0 = H - bas, h = v => v / plafond * (H - haut - bas);
    el('line', { x1: MARGES.g, x2: W - MARGES.d, y1: haut, y2: haut, class: 'grille' }, svg);
    el('line', { x1: MARGES.g, x2: W - MARGES.d, y1: y0, y2: y0, class: 'grille' }, svg);
    texte(svg, MARGES.g - 6, haut + 4, `${plafond}`, 'axe', 'end');
    texte(svg, MARGES.g - 6, y0 + 4, '0', 'axe', 'end');
    texte(svg, W - MARGES.d + 6, haut + 4, 'mm/h', 'axe');
    const pas = x(K0 + 1) - x(K0), l = Math.max(1, Math.min(24, pas - 2));
    ks.forEach((k, i) => {
      const hb = h(mm[i]);
      if (hb <= 0) return;
      const g = x(k) - l / 2, r = Math.min(4, l / 2, hb), yt = y0 - hb;
      // bout arrondi en haut, carré sur la ligne de base
      el('path', { d: `M${g},${y0}V${yt + r}Q${g},${yt} ${g + r},${yt}H${g + l - r}Q${g + l},${yt} ${g + l},${yt + r}V${y0}Z`, fill: C1 }, svg);
    });
    brancherSurvol(svg);
  }

  function brancherSurvol(svg) {
    svg.onpointermove = e => {
      const r = svg.getBoundingClientRect(), px = (e.clientX - r.left) * (geo.W / r.width);
      const k = Math.round(K0 + (px - MARGES.g) / (geo.W - MARGES.g - MARGES.d) * (K1 - K0));
      montrer(Math.max(geo.ks[0], Math.min(geo.ks[geo.ks.length - 1], k)));
    };
    svg.onpointerleave = cacher;
  }

  function ligneBulle(parent, couleur, libelle, valeur) {
    const d = document.createElement('div');
    if (couleur) { const i = document.createElement('i'); i.style.setProperty('--c', couleur); d.append(i); }
    d.append(libelle);
    if (valeur) { const b = document.createElement('b'); b.textContent = valeur; d.append(' ', b); }
    parent.append(d);
  }

  function montrer(k) {
    const { s, x, y, curseur } = geo, m = s[k];
    if (!m) return;
    const [ligne, d1, d2] = curseur.children;
    for (const e of [ligne]) { e.setAttribute('x1', x(k)); e.setAttribute('x2', x(k)); }
    d1.setAttribute('cx', x(k)); d1.setAttribute('cy', y(m.p_cumul));
    d2.setAttribute('cx', x(k)); d2.setAttribute('cy', y(m.p));
    curseur.setAttribute('visibility', 'visible');
    const b = $('#bulle');
    b.replaceChildren();
    ligneBulle(b, null, `Dans ${k} min (${hm(m.t)})`);
    ligneBulle(b, C1, 'à cette minute', pct(m.p));
    ligneBulle(b, C2, "d'ici là", pct(m.p_cumul));
    if (m.mm_h > 0) ligneBulle(b, null, 'si pluie :', `${mmTexte(m.mm_h)} (${force(m.mm_h)})`);
    ligneBulle(b, null, `radar ${pct(m.p_radar)}${m.p_arome != null ? ` · AROME ${pct(m.p_arome)}` : ''}`);
    b.hidden = false;
    const svg = $('#courbe'), fig = $('#fig-p'), echelle = svg.getBoundingClientRect().width / geo.W;
    const gauche = svg.offsetLeft + x(k) * echelle;
    b.style.left = `${gauche + 12 + b.offsetWidth > fig.clientWidth ? gauche - 12 - b.offsetWidth : gauche + 12}px`;
    b.style.top = `${svg.offsetTop + 6}px`;
  }

  function cacher() {
    $('#bulle').hidden = true;
    if (geo.curseur) geo.curseur.setAttribute('visibility', 'hidden');
  }

  function remplirTableau(s) {
    const corps = $('#tableau tbody');
    corps.replaceChildren();
    for (const k of [5, 10, 15, 20, 30, 45, 60]) {
      const m = s[k];
      if (!m) continue;
      const tr = document.createElement('tr');
      for (const v of [`${k} min`, pct(m.p), pct(m.p_cumul), m.mm_h > 0 ? mmTexte(m.mm_h) : '—', pct(m.p_radar), m.p_arome != null ? pct(m.p_arome) : '—']) {
        const td = document.createElement('td');
        td.textContent = v;
        tr.append(td);
      }
      corps.append(tr);
    }
  }

  // ---------- Notifications ----------
  if ('serviceWorker' in navigator && window.isSecureContext) navigator.serviceWorker.register('/sw.js').catch(() => {});
  function notifier(titre, corps) {
    if (!('Notification' in window) || Notification.permission !== 'granted') return;
    const sw = navigator.serviceWorker;
    (sw ? sw.getRegistration() : Promise.resolve(null))
      .then(reg => (reg ? reg.showNotification(titre, { body: corps, tag: 'pluie' }) : new Notification(titre, { body: corps })))
      .catch(() => {});
  }

  function verifierAlerte(p) {
    const s = p.serie;
    if (!R.notifs || s.length < 2) return;
    const c = s[Math.min(30, s.length - 1)].p_cumul;
    if (!etat.alerte && c >= R.seuil && s[0].p < 0.5) {
      etat.alerte = true; // une alerte par épisode : la suivante attend que le risque retombe
      const k = s.findIndex(m => m.p >= 0.5);
      notifier(p.point.neige ? 'Neige en approche' : 'Pluie en approche',
        k >= 0 ? `Probable dans ~${k} min (${pct(s[k].p)}).` : `Risque d'ici 30 min : ${pct(c)}.`);
    } else if (etat.alerte && c < R.seuil / 2) {
      etat.alerte = false;
    }
  }

  // ---------- Réglages ----------
  $('#o-fleches').checked = R.fleches;
  $('#o-cellules').checked = R.cellules;
  $('#o-notifs').checked = R.notifs && 'Notification' in window && Notification.permission === 'granted';
  $('#o-seuil').value = String(R.seuil);
  $('#o-fleches').addEventListener('change', e => { R.fleches = e.target.checked; garder(); majCouches(); });
  $('#o-cellules').addEventListener('change', e => { R.cellules = e.target.checked; garder(); majCouches(); });
  $('#o-seuil').addEventListener('change', e => { R.seuil = +e.target.value; etat.alerte = false; garder(); });
  $('#o-notifs').addEventListener('change', async e => {
    if (e.target.checked) {
      if (!('Notification' in window)) {
        e.target.checked = false;
        $('#detail').textContent = "Notifications non prises en charge ici (sur iPhone : ajouter l'appli à l'écran d'accueil).";
        return;
      }
      if (Notification.permission !== 'granted' && (await Notification.requestPermission().catch(() => 'denied')) !== 'granted') {
        e.target.checked = false;
        $('#detail').textContent = 'Notifications refusées par le navigateur.';
        return;
      }
    }
    R.notifs = e.target.checked;
    garder();
    if (R.notifs) notifier('Notifications activées', 'Vous serez prévenu quand la pluie arrive.');
  });
  window.addEventListener('resize', () => { if (etat.prev && etat.prev.serie.length > K0) { dessinerCourbe(etat.prev.serie); dessinerBarres(etat.prev.serie); } });
  document.addEventListener('visibilitychange', () => { if (!document.hidden) charger(); });

  majCouches();
  suivreGps();
  charger();
})();

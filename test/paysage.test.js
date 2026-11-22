// Banc du fond d'écran (web/js/paysage.js) : moment de la journée, temps choisi, décor, particules.
'use strict';
const assert = require('assert');
const P = require('../web/js/paysage.js');

const lever = 1_800_000_000, coucher = lever + 9 * 3600;
assert.strictEqual(P.moment(lever - 4000, lever, coucher), 'nuit');
assert.strictEqual(P.moment(lever + 600, lever, coucher), 'aube');
assert.strictEqual(P.moment(lever + 4 * 3600, lever, coucher), 'jour');
assert.strictEqual(P.moment(coucher - 600, lever, coucher), 'crepuscule');
assert.strictEqual(P.moment(coucher + 4000, lever, coucher), 'nuit');

const prev = (dbz, ciel, ps = [], neige = false) => ({ point: { dbz, neige }, ciel,
  serie: Array.from({ length: 61 }, (_, k) => ({ p: ps[k] || 0, mm_h: dbz >= 12 ? 2.5 : 0 })) });
assert.deepStrictEqual(P.meteoDe(null), { meteo: 'clair', intensite: 'moyenne' });
assert.deepStrictEqual(P.meteoDe(prev(30, { nuages: 10 })), { meteo: 'pluie', intensite: 'moyenne' }, 'le radar prime sur les nuages');
assert.strictEqual(P.meteoDe(prev(30, {}, [], true)).meteo, 'neige');
assert.strictEqual(P.meteoDe(prev(48, {})).meteo, 'orage');
assert.strictEqual(P.meteoDe(prev(30, { code: 95 })).meteo, 'orage');
assert.strictEqual(P.meteoDe(prev(null, { nuages: 10 })).meteo, 'clair');
assert.strictEqual(P.meteoDe(prev(null, { nuages: 60 })).meteo, 'nuageux');
assert.strictEqual(P.meteoDe(prev(null, { nuages: 95 })).meteo, 'couvert');
assert.strictEqual(P.meteoDe(prev(null, { code: 45, nuages: 100 })).meteo, 'brouillard');
assert.strictEqual(P.meteoDe(prev(null, { code: 61 })).meteo, 'couvert', 'le modèle fait pleuvoir, le radar non : couvert');
assert.strictEqual(P.meteoDe(prev(null, { nuages: 10 }, [0, 0, 0, 0, 0, 0, 0, 0, 0.6])).meteo, 'couvert', 'pluie dans 8 min : le ciel se charge');
assert.strictEqual(P.meteoDe(prev(5, null)).meteo, 'clair', 'écho trop faible pour mouiller');

const d = P.decor('jour', 'pluie', false);
assert.strictEqual(d.astre, null);
assert.ok(d.fenetres, 'fenêtres allumées sous la pluie');
assert.strictEqual(P.decor('nuit', 'clair').astre, 'lune');
assert.ok(P.decor('nuit', 'clair').etoiles);
assert.strictEqual(P.decor('inconnu', 'grêle').meteo, 'clair');
for (const m of P.MOMENTS) for (const t of P.METEOS) {
  const s = P.dessiner(P.decor(m, t));
  assert.ok(s.startsWith('<svg') && s.endsWith('</svg>') && !s.includes('undefined'), `${m}/${t}`);
}
assert.deepStrictEqual(P.particulesPour('pluie', 'forte', false), { type: 'pluie', nombre: 240 });
assert.strictEqual(P.particulesPour('clair', 'moyenne', false), null);
assert.strictEqual(P.particulesPour('clair', 'moyenne', true).type, 'etoiles');
console.log('paysage : ok');

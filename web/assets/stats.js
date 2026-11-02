/* eduquery — статистика, графики, записи в карточке ученика. */
'use strict';

// ─────────────────────────────── Подсказка при наведении ───────────────────────────────

(function initTip() {
  const tip = () => document.getElementById('tip');
  document.addEventListener('mousemove', (e) => {
    const t = tip();
    if (!t) return;
    const el = e.target.closest ? e.target.closest('[data-tip]') : null;
    if (!el) { t.style.display = 'none'; return; }
    t.textContent = el.getAttribute('data-tip');
    t.style.display = 'block';
    const w = t.offsetWidth;
    const x = Math.min(e.clientX + 14, window.innerWidth - w - 8);
    t.style.left = Math.max(8, x) + 'px';
    t.style.top = (e.clientY + 16) + 'px';
  });
  document.addEventListener('mouseleave', () => { const t = tip(); if (t) t.style.display = 'none'; });
})();

// ─────────────────────────────── Графики ───────────────────────────────

const fmt1 = (v) => (v == null ? '—' : (Math.round(v * 10) / 10).toString().replace('.', ','));
const fmtInt = (v) => String(Math.round(v));
const monthLabel = (p) => {
  const m = ['янв', 'фев', 'мар', 'апр', 'май', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек'];
  const [y, mo] = String(p).split('-');
  return (m[(+mo || 1) - 1] || p) + ' ' + String(y).slice(2);
};
const emptyChart = () => '<div class="chart-empty">Данных пока нет</div>';

/** Ранжированные горизонтальные столбцы. items: [{label, value, tip?, id?}] */
function hbars(items, { fmt = fmtInt, max = null, onClick = null, color = null } = {}) {
  if (!items.length) return emptyChart();
  const top = max != null ? max : Math.max(...items.map((i) => i.value), 1e-9);
  return items.map((i, n) => {
    const w = Math.max(0, Math.min(100, (i.value / top) * 100));
    const tip = esc(i.tip || `${i.label}: ${fmt(i.value)}`);
    return `<div class="hb-row ${onClick ? 'click' : ''}" data-tip="${tip}" ${onClick ? `onclick="${onClick}(${i.id})"` : ''}>
      <span class="hb-l">${esc(i.label)}</span>
      <div class="hb-t"><i style="width:${w}%;${color ? `background:${color}` : ''}"></i></div>
      <span class="hb-v">${fmt(i.value)}</span></div>`;
  }).join('');
}

/** Колонки; series: [{name,color}], rows: [{label, values:[...]}] — составные при >1 серии. */
function columns(rows, series, { fmt = fmtInt } = {}) {
  if (!rows.length) return emptyChart();
  const totals = rows.map((r) => r.values.reduce((a, b) => a + b, 0));
  const top = Math.max(...totals, 1);
  const H = 130; // px под столбцом
  const legend = series.length > 1
    ? `<div class="legend">${series.map((s) => `<span><i style="background:${s.color}"></i>${esc(s.name)}</span>`).join('')}</div>` : '';
  const cols = rows.map((r, n) => {
    const tip = esc(`${r.label}\n` + series.map((s, k) => `${s.name}: ${fmt(r.values[k])}`).join('\n') +
      (series.length > 1 ? `\nВсего: ${fmt(totals[n])}` : ''));
    const segs = r.values.map((v, k) =>
      v > 0 ? `<div class="seg" style="height:${(v / top) * H}px;background:${series[k].color}"></div>` : '').join('');
    return `<div class="col" data-tip="${tip}">${totals[n] > 0 ? `<div class="tot" style="bottom:${(totals[n] / top) * H}px">${fmt(totals[n])}</div>` : ''}${segs}</div>`;
  }).join('');
  const labels = rows.map((r) => `<span>${esc(r.label)}</span>`).join('');
  return `${legend}<div class="cols">${cols}</div><div class="col-labels">${labels}</div>`;
}

/** Линейный график одной серии. points: [{label, value}] */
function lineChart(points, { min = null, max = null, fmt = fmt1, unit = '' } = {}) {
  const pts = points.filter((p) => p.value != null);
  if (pts.length < 1) return emptyChart();
  const W = 560, H = 190, L = 34, R = 14, T = 14, B = 26;
  const vals = pts.map((p) => p.value);
  let lo = min != null ? min : Math.min(...vals);
  let hi = max != null ? max : Math.max(...vals);
  if (hi - lo < 1e-9) { lo -= 1; hi += 1; }
  const x = (i) => L + (pts.length === 1 ? (W - L - R) / 2 : (i / (pts.length - 1)) * (W - L - R));
  const y = (v) => T + (1 - (v - lo) / (hi - lo)) * (H - T - B);
  const ticks = 4;
  let grid = '';
  for (let k = 0; k <= ticks; k++) {
    const v = lo + ((hi - lo) * k) / ticks;
    grid += `<line class="grid" x1="${L}" x2="${W - R}" y1="${y(v)}" y2="${y(v)}"/>` +
      `<text x="${L - 6}" y="${y(v) + 3}" text-anchor="end">${fmt(v)}</text>`;
  }
  const path = pts.map((p, i) => `${i ? 'L' : 'M'}${x(i)},${y(p.value)}`).join(' ');
  const area = `${path} L${x(pts.length - 1)},${H - B} L${x(0)},${H - B} Z`;
  const dots = pts.map((p, i) =>
    `<circle class="dot" cx="${x(i)}" cy="${y(p.value)}" r="4" data-tip="${esc(p.label)}: ${fmt(p.value)}${unit}"/>`).join('');
  const step = Math.ceil(pts.length / 8);
  const xl = pts.map((p, i) => (i % step === 0
    ? `<text x="${x(i)}" y="${H - 8}" text-anchor="middle">${esc(p.label)}</text>` : '')).join('');
  return `<svg class="line-svg" viewBox="0 0 ${W} ${H}" role="img">${grid}<path class="area" d="${area}"/><path class="ln" d="${path}"/>${dots}${xl}</svg>`;
}

// ─────────────────────────────── Вкладка «Статистика» ───────────────────────────────

let statsData = null;

async function loadStats() {
  try {
    const sel = $('stGroup');
    if (!sel.dataset.loaded) {
      const gr = await api('/api/v1/groups');
      gr.groups.forEach((g) => {
        const o = document.createElement('option');
        o.value = g; o.textContent = g;
        sel.appendChild(o);
      });
      sel.dataset.loaded = '1';
    }
    statsData = await api('/api/v1/stats?group=' + encodeURIComponent(sel.value));
    renderStats();
  } catch (e) {
    toast('Ошибка статистики: ' + e.message, 'err');
  }
}

function renderStats() {
  const d = statsData;
  if (!d) return;
  const t = d.totals;
  const mealsPct = t.meals ? (t.meals_eaten / t.meals) * 100 : null;
  $('stCards').innerHTML = [
    ['Учеников', t.students, d.group ? 'в выбранной группе' : 'всего'],
    ['Средняя оценка', fmt1(t.avg_grade), 'по 5-балльной шкале'],
    ['Пропущено уроков', t.absences, `без уважительной причины: ${t.absences_unexcused}`],
    ['Портфолио', t.portfolio, 'грамоты, достижения, благодарности'],
    ['Поведение', `${t.praises} / ${t.remarks}`, 'поощрений / замечаний и нарушений'],
    ['Питание', mealsPct == null ? '—' : fmt1(mealsPct) + '%', 'дней, когда питались'],
  ].map(([k, v, s]) => `<div class="card"><div class="k">${k}</div><div class="v">${v}</div><div class="s">${s}</div></div>`).join('');

  // Оценки: колонки 2…5
  const dist = [2, 3, 4, 5].map((g) => ({
    label: String(g),
    values: [(d.grade_dist.find((x) => x.grade === g) || { count: 0 }).count],
  }));
  $('chGradeDist').innerHTML = d.grade_dist.length
    ? columns(dist, [{ name: 'Оценок', color: 'var(--seq)' }]) : emptyChart();

  $('chGradeSubj').innerHTML = hbars(
    d.grade_by_subject.map((s) => ({ label: s.subject, value: s.avg_grade, tip: `${s.subject}: ${fmt1(s.avg_grade)} (оценок: ${s.count})` })),
    { fmt: fmt1, max: 5 });

  $('chAbsReason').innerHTML = hbars(
    d.absences_by_reason.map((r) => ({ label: r.reason, value: r.lessons })));

  $('chAbsMonth').innerHTML = columns(
    d.absences_by_month.map((m) => ({ label: monthLabel(m.period), values: [m.excused, m.unexcused] })),
    [{ name: 'По уважительной причине', color: 'var(--c1)' }, { name: 'Без уважительной причины', color: 'var(--c2)' }]);

  $('chPortCat').innerHTML = hbars(d.portfolio_by_category.map((r) => ({ label: r.category, value: r.count })));
  const order = ['Школьный', 'Муниципальный', 'Региональный', 'Всероссийский', 'Международный'];
  $('chPortLvl').innerHTML = hbars(
    d.portfolio_by_level.slice().sort((a, b) => order.indexOf(a.level) - order.indexOf(b.level))
      .map((r) => ({ label: r.level, value: r.count })));

  $('chBeh').innerHTML = columns(
    d.behavior_by_month.map((m) => ({ label: monthLabel(m.period), values: [m.praise, m.remark, m.violation] })),
    [{ name: 'Поощрения', color: 'var(--c1)' }, { name: 'Замечания', color: 'var(--c2)' }, { name: 'Нарушения', color: 'var(--c3)' }]);

  const mealRows = [
    ...d.meals_by_type.map((r) => ({ label: r.meal, eaten: r.eaten, total: r.eaten + r.skipped })),
    ...d.meals_by_payment.map((r) => ({ label: r.payment + ' питание', eaten: r.eaten, total: r.eaten + r.skipped })),
  ].filter((r) => r.total > 0).map((r) => ({
    label: r.label, value: (r.eaten / r.total) * 100,
    tip: `${r.label}: питался ${r.eaten} из ${r.total}`,
  }));
  $('chMeals').innerHTML = hbars(mealRows, { fmt: (v) => fmt1(v) + '%', max: 100 });

  $('chGradeTrend').innerHTML = lineChart(
    d.by_month.map((m) => ({ label: monthLabel(m.period), value: m.avg_grade })), { min: 2, max: 5 });
  $('chRatioTrend').innerHTML = lineChart(
    d.by_month.map((m) => ({ label: monthLabel(m.period), value: m.avg_ratio == null ? null : m.avg_ratio * 100 })),
    { min: 0, max: 100, fmt: fmtInt, unit: '%' });

  renderGroupChart();
  renderRank();
}

function renderGroupChart() {
  if (!statsData) return;
  const m = $('stGroupMetric').value;
  const rows = statsData.by_group.map((g) => ({
    ...g, meals_pct: g.meals ? (g.meals_eaten / g.meals) * 100 : null,
  })).filter((g) => g[m] != null);
  const isAvg = m === 'avg_grade';
  $('chGroups').innerHTML = hbars(
    rows.sort((a, b) => b[m] - a[m]).map((g) => ({
      label: `${g.group} (${g.students} чел.)`, value: g[m],
    })),
    { fmt: m === 'meals_pct' ? (v) => fmt1(v) + '%' : (isAvg || m === 'absences_per_student' ? fmt1 : fmtInt), max: isAvg ? 5 : null });
}

function renderRank() {
  if (!statsData) return;
  const k = $('stRank').value;
  const list = statsData.students.map((s) => ({ ...s, meals_pct: s.meals ? (s.meals_eaten / s.meals) * 100 : null }));
  let key = k, asc = false;
  if (k === 'avg_grade_hi') key = 'avg_grade';
  if (k === 'avg_grade_lo') { key = 'avg_grade'; asc = true; }
  if (k === 'meals_pct_lo') { key = 'meals_pct'; asc = true; }
  const rows = list.filter((s) => s[key] != null)
    .sort((a, b) => (asc ? a[key] - b[key] : b[key] - a[key]))
    .filter((s) => (asc || key === 'avg_grade' ? true : s[key] > 0))
    .slice(0, 10);
  const real = key === 'avg_grade' || key === 'meals_pct';
  $('chRank').innerHTML = hbars(
    rows.map((s) => ({
      id: s.id, label: s.full_name, value: s[key],
      tip: `${s.full_name} (${s.group_name || s.external_id})\nЗамечаний и нарушений: ${s.remarks}, поощрений: ${s.praises}\nПропусков: ${s.absences} (без уважительной: ${s.absences_unexcused})\nСредняя оценка: ${fmt1(s.avg_grade)}`,
    })),
    {
      fmt: key === 'meals_pct' ? (v) => fmt1(v) + '%' : (real ? fmt1 : fmtInt),
      max: key === 'avg_grade' ? 5 : (key === 'meals_pct' ? 100 : null),
      onClick: 'openStudent',
    });
}

// ─────────────────────────────── Карточка ученика: разделы ───────────────────────────────

const canImport = () => typeof can === 'function' && can('import');
const delBtn = (id) => (canImport() ? `<button class="rec-del" title="Удалить запись" onclick="deleteRecord(${id})">×</button>` : '');
const day10 = (s) => String(s || '').slice(0, 10);

function studentExtras(d) {
  let html = '';
  const add = canImport()
    ? `<button class="b b-sm b-p" onclick="openRecord(${d.summary.id})">+ Добавить запись</button>` : '';
  html += `<div class="sec" style="display:flex;gap:8px;align-items:center;justify-content:space-between">
    <div class="sec-t" style="margin:0;border:none;padding:0">Оценки, пропуски, портфолио, поведение, питание</div>${add}</div>`;

  // Оценки
  html += `<div class="sec"><div class="sec-t">Оценки${d.summary.avg_grade != null ? ` — средняя ${fmt1(d.summary.avg_grade)}` : ''}</div>`;
  if (d.grades.length) {
    html += `<table><thead><tr><th>Предмет</th><th class="num">Средняя</th><th class="num">Оценок</th><th>Последние</th></tr></thead><tbody>` +
      d.grades.map((g) => `<tr><td>${esc(g.subject)}</td><td class="num">${fmt1(g.avg_grade)}</td><td class="num">${g.count}</td>
        <td><div class="chips">${g.last.map((x) => `<span class="chip g${x}">${x}</span>`).join('')}</div></td></tr>`).join('') +
      '</tbody></table>';
  } else html += '<div class="muted">Оценок нет.</div>';
  html += '</div>';

  // Пропуски
  const a = d.absence;
  html += `<div class="sec"><div class="sec-t">Пропуски — ${a.total} ур. (без уважительной причины: ${a.unexcused})</div>`;
  if (a.items.length) {
    html += hbars(a.by_reason.map((r) => ({ label: r.reason, value: r.lessons })), { color: 'var(--seq)' });
    html += `<div class="rec-list list" style="margin-top:8px">` + a.items.slice(0, 12).map((it) => `<div class="list-row">
      <span class="muted">${esc(day10(it.occurred_at))}</span>
      <span class="grow">${it.amount} ур.${it.subject ? ' · ' + esc(it.subject) : ''} · ${esc(it.reason || 'причина не указана')}
        ${it.comment ? `<span class="muted"> (${esc(it.comment)})</span>` : ''}</span>
      <span class="tag ${it.status === 'excused' ? 'tag-ok' : 'tag-r'}">${it.status === 'excused' ? 'уважительная' : 'без уваж. причины'}</span>${delBtn(it.id)}</div>`).join('') + '</div>';
  } else html += '<div class="muted">Пропусков нет.</div>';
  html += '</div>';

  // Портфолио
  html += `<div class="sec"><div class="sec-t">Портфолио — ${d.portfolio_items.length}</div>`;
  if (d.portfolio_items.length) {
    html += `<div class="rec-list list">` + d.portfolio_items.map((it) => `<div class="list-row">
      <span class="muted">${esc(day10(it.occurred_at))}</span>
      <span class="grow"><b>${esc(it.title)}</b>${it.comment ? `<span class="muted"> · ${esc(it.comment)}</span>` : ''}</span>
      ${it.category ? `<span class="tag tag-a">${esc(it.category)}</span>` : ''}${it.level ? `<span class="tag">${esc(it.level)}</span>` : ''}${delBtn(it.id)}</div>`).join('') + '</div>';
  } else html += '<div class="muted">Записей нет.</div>';
  html += '</div>';

  // Поведение
  const b = d.behavior;
  html += `<div class="sec"><div class="sec-t">Поведение — поощрений ${b.praises}, замечаний и нарушений ${b.remarks}</div>`;
  if (b.items.length) {
    html += `<div class="rec-list list">` + b.items.slice(0, 15).map((it) => {
      const cls = it.category === 'Поощрение' ? 'tag-ok' : (it.category === 'Нейтрально' ? '' : 'tag-r');
      return `<div class="list-row"><span class="muted">${esc(day10(it.occurred_at))}</span>
        <span class="grow">${esc(it.comment || it.title)}</span><span class="tag ${cls}">${esc(it.category || 'запись')}</span>${delBtn(it.id)}</div>`;
    }).join('') + '</div>';
  } else html += '<div class="muted">Записей нет.</div>';
  html += '</div>';

  // Питание
  const m = d.meals;
  html += `<div class="sec"><div class="sec-t">Питание${m.payment ? ' — ' + esc(m.payment.toLowerCase()) : ''}</div>`;
  if (m.total) {
    html += `<div class="muted" style="margin-bottom:6px">Питался ${m.eaten} из ${m.total} (${fmt1((m.eaten / m.total) * 100)}%)</div>` +
      hbars(m.by_type.map((t) => ({
        label: t.meal, value: (t.eaten / Math.max(1, t.eaten + t.skipped)) * 100,
        tip: `${t.meal}: питался ${t.eaten}, не питался ${t.skipped}`,
      })), { fmt: (v) => fmt1(v) + '%', max: 100 });
  } else html += '<div class="muted">Данных о питании нет.</div>';
  html += '</div>';
  return html;
}

// ─────────────────────────────── Добавление / удаление записей ───────────────────────────────

let recordStudentId = null;

function rcKindChanged() {
  const k = $('rcKind').value;
  document.querySelectorAll('#mRecord [data-for]').forEach((el) => {
    el.classList.toggle('hide', !el.dataset.for.split(' ').includes(k));
  });
}

function openRecord(studentId) {
  recordStudentId = studentId;
  $('rcDate').value = new Date().toISOString().slice(0, 10);
  $('rcMsg').textContent = '';
  rcKindChanged();
  $('mRecord').classList.add('on');
}

function closeRecord() { $('mRecord').classList.remove('on'); }

async function saveRecord() {
  const k = $('rcKind').value;
  const body = { kind: k, occurred_at: $('rcDate').value || undefined };
  const v = (id) => $(id).value.trim();
  if (k === 'grade') Object.assign(body, { subject: v('rcSubject'), title: v('rcTitle'), grade: +$('rcGrade').value, comment: v('rcComment') });
  if (k === 'absence') Object.assign(body, { subject: v('rcSubject'), amount: +$('rcAmount').value || 1, reason: v('rcReason'), excused: $('rcExcused').value, comment: v('rcComment') });
  if (k === 'portfolio') Object.assign(body, { title: v('rcTitle'), category: v('rcCatPort'), level: $('rcLevel').value, comment: v('rcComment') });
  if (k === 'behavior') Object.assign(body, { category: $('rcCatBeh').value, comment: v('rcComment') });
  if (k === 'meal') Object.assign(body, { category: $('rcMeal').value, status: $('rcAte').value, level: $('rcPay').value });
  if (['task', 'quiz', 'submission'].includes(k)) {
    Object.assign(body, { subject: v('rcSubject'), title: v('rcTitle'), score: $('rcScore').value, max_score: $('rcMax').value, status: $('rcStatus').value });
  }
  if (k === 'grade' && !(body.grade >= 1 && body.grade <= 5)) {
    $('rcMsg').innerHTML = '<span style="color:var(--bad)">Оценка должна быть от 1 до 5</span>';
    return;
  }
  try {
    await api(`/api/v1/students/${recordStudentId}/records`, { method: 'POST', body });
    closeRecord();
    toast('Запись добавлена', 'ok');
    openStudent(recordStudentId);
    loadStudents();
  } catch (e) {
    $('rcMsg').className = 'output';
    $('rcMsg').innerHTML = '<span style="color:var(--bad)">' + esc(e.message) + '</span>';
  }
}

async function deleteRecord(id) {
  if (!confirm('Удалить запись?')) return;
  try {
    await api('/api/v1/records/' + id, { method: 'DELETE' });
    toast('Запись удалена', 'ok');
    if (state.currentStudent) openStudent(state.currentStudent.summary.id);
    loadStudents();
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

function downloadTemplate() { download('/api/v1/import/template.csv'); }

async function resetSystemPrompt() {
  try {
    const p = await api('/api/v1/params');
    $('pSystem').value = p.default_system_prompt || '';
    toast('Стандартный промпт подставлен — нажмите «Сохранить»', 'ok');
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

document.addEventListener('DOMContentLoaded', () => {
  const m = $('mRecord');
  if (m) m.addEventListener('click', (e) => { if (e.target === m) closeRecord(); });
  rcKindChanged();
});

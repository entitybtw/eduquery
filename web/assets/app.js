/* eduquery — клиентская логика веб-интерфейса. */
'use strict';

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

const state = {
  students: [],
  models: [],
  catalog: [],
  activeModel: '',
  currentStudent: null,
  theme: localStorage.getItem('eq-theme') || 'dark',
  accent: localStorage.getItem('eq-accent') || '#7a9db8',
};

// ─────────────────────────────── Утилиты ───────────────────────────────

function toast(msg, kind = '') {
  const t = document.createElement('div');
  t.className = 'toast ' + kind;
  t.textContent = msg;
  $('toasts').appendChild(t);
  setTimeout(() => t.remove(), 4200);
}

async function api(path, opts = {}) {
  const o = Object.assign({ headers: {} }, opts);
  if (o.body && typeof o.body !== 'string') {
    o.headers['Content-Type'] = 'application/json';
    o.body = JSON.stringify(o.body);
  }
  const r = await fetch(path, o);
  if (r.status === 401) {
    showLogin();
    throw new Error('сессия истекла');
  }
  const text = await r.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch { data = { raw: text }; }
  if (!r.ok) throw new Error((data && data.error) || `HTTP ${r.status}`);
  return data;
}

function fmtBytes(n) {
  n = Number(n) || 0;
  if (n < 1024) return n + ' Б';
  if (n < 1048576) return (n / 1024).toFixed(1) + ' КБ';
  if (n < 1073741824) return (n / 1048576).toFixed(1) + ' МБ';
  return (n / 1073741824).toFixed(2) + ' ГБ';
}

function pct(v) {
  if (v === null || v === undefined) return '—';
  return Math.round(v * 100) + '%';
}

function barClass(v) {
  if (v === null || v === undefined) return '';
  if (v >= 0.8) return 'g';
  if (v >= 0.5) return 'w';
  return 'r';
}

function fmtDate(s) {
  if (!s) return '—';
  return String(s).replace('T', ' ').slice(0, 16);
}

function busy(el, text = 'Генерация…') {
  el.innerHTML = '<div class="busy"><div class="spin"></div>' + esc(text) + '</div>';
}

function mdToHtml(src) {
  if (!src) return '';
  // Минимальный markdown-рендерер: заголовки, списки, блоки кода, inline-код, жирный.
  const blocks = [];
  let text = String(src).replace(/```(\w*)\n([\s\S]*?)```/g, (m, lang, code) => {
    blocks.push('<pre><code>' + esc(code) + '</code></pre>');
    return '@@BLOCK' + (blocks.length - 1) + '@@';
  });

  const lines = text.split('\n');
  let html = '';
  let inList = false;

  const inline = (s) => esc(s)
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|\s)\*([^*\n]+)\*/g, '$1<em>$2</em>');

  const closeList = () => { if (inList) { html += '</ul>'; inList = false; } };

  for (const raw of lines) {
    const line = raw.trimEnd();
    if (/^@@BLOCK\d+@@$/.test(line.trim())) {
      closeList();
      html += blocks[+line.trim().match(/\d+/)[0]];
      continue;
    }
    if (!line.trim()) { closeList(); continue; }
    let m;
    if ((m = line.match(/^#{1,6}\s+(.*)$/))) {
      closeList();
      const lvl = Math.min(3, m[1].length);
      html += `<h${lvl}>${inline(m[2])}</h${lvl}>`;
      continue;
    }
    if ((m = line.match(/^\s*[-*]\s+(.*)$/))) {
      if (!inList) { html += '<ul>'; inList = true; }
      html += `<li>${inline(m[1])}</li>`;
      continue;
    }
    if ((m = line.match(/^\s*(\d+)[.)]\s+(.*)$/))) {
      if (!inList) { html += '<ol>'; inList = true; }
      html += `<li>${inline(m[2])}</li>`;
      continue;
    }
    if ((m = line.match(/^>\s?(.*)$/))) {
      closeList();
      html += `<blockquote>${inline(m[1])}</blockquote>`;
      continue;
    }
    if (/^---+$/.test(line.trim())) { closeList(); html += '<hr>'; continue; }
    closeList();
    html += `<p>${inline(line)}</p>`;
  }
  closeList();
  return html;
}

function setOutput(el, text, loopDetected) {
  el.className = 'output md';
  el.innerHTML = mdToHtml(text);
  if (loopDetected) {
    el.insertAdjacentHTML('afterbegin',
      '<div class="notice">Модель ушла в повтор — генерация прервана. Попробуй другую модель или снизь температуру.</div>');
  }
}

// ─────────────────────────────── Вкладки ───────────────────────────────

function tab(name) {
  document.querySelectorAll('.ntb').forEach((b) =>
    b.classList.toggle('a', b.dataset.tab === name));
  ['dashboard', 'students', 'analyze', 'models', 'settings'].forEach((t) => {
    $('p-' + t).style.display = t === name ? '' : 'none';
  });
  if (name === 'dashboard') loadOverview();
  if (name === 'students') loadStudents();
  if (name === 'analyze') { loadStudents(); syncStudentSelects(); }
  if (name === 'models') loadModels();
  if (name === 'settings') loadParams();
}

// ─────────────────────────────── Тема ───────────────────────────────

function applyAccent(hex) {
  if (!/^#[0-9a-fA-F]{3,8}$/.test(hex)) return;
  document.documentElement.style.setProperty('--accent', hex);
  document.documentElement.style.setProperty('--ag', hex + '1a');
  localStorage.setItem('eq-accent', hex);
  state.accent = hex;
  const p = $('acPick'); if (p) p.value = hex;
}

function setTheme(t) {
  document.documentElement.dataset.theme = t;
  localStorage.setItem('eq-theme', t);
  state.theme = t;
  const s = $('themeSel'); if (s) s.value = t;
}

function cycleTheme() {
  const cs = ['dark', 'light', 'black'];
  setTheme(cs[(cs.indexOf(state.theme) + 1) % cs.length]);
}

// ─────────────────────────────── Авторизация ───────────────────────────────

function showLogin() {
  $('login').style.display = '';
  $('app').style.display = 'none';
}

async function doLogin() {
  const u = $('iUser').value.trim();
  const p = $('iPass').value;
  $('loginHint').textContent = '';
  try {
    const r = await api('/api/v1/login', { method: 'POST', body: { username: u, password: p } });
    localStorage.setItem('eq-token', r.token);
    $('iPass').value = '';
    await boot();
  } catch (e) {
    $('loginHint').textContent = e.message || 'ошибка входа';
  }
}

async function doLogout() {
  try { await api('/api/v1/logout', { method: 'POST' }); } catch { /* уже вышли */ }
  localStorage.removeItem('eq-token');
  showLogin();
}

async function boot() {
  const me = await api('/api/v1/me');
  $('login').style.display = 'none';
  $('app').style.display = '';
  state.activeModel = me.active_model || '';
  updateActiveLabel();
  await Promise.all([loadOverview(), loadModels()]);
}

function updateActiveLabel() {
  const el = $('activeModel');
  el.textContent = state.activeModel
    ? (state.activeModel.split('/').pop().replace(/\.gguf$/, '') || '—')
    : 'не выбрана';
  el.style.color = state.activeModel ? 'var(--accent)' : 'var(--bad)';
}

// ─────────────────────────────── Обзор ───────────────────────────────

async function loadOverview() {
  try {
    const o = await api('/api/v1/overview');
    $('ovCards').innerHTML = [
      ['Учеников', o.students, 'в потоке'],
      ['Активностей', o.activities, 'всего записей'],
      ['Средний результат', pct(o.avg_ratio), 'по всем работам'],
      ['Время на ученика', o.avg_minutes != null ? Math.round(o.avg_minutes) + ' мин' : '—', 'суммарно'],
    ].map(([k, v, s]) => `<div class="card"><div class="k">${k}</div><div class="v">${v}</div><div class="s">${s}</div></div>`).join('');

    $('ovGroups').innerHTML = o.top_groups.length ? `<table><thead><tr><th>Группа</th><th class="num">Чел.</th><th class="num">Результат</th><th style="width:120px"></th></tr></thead><tbody>` +
      o.top_groups.map((g) => `<tr>
        <td>${esc(g.group_name)}</td>
        <td class="num">${g.students}</td>
        <td class="num">${pct(g.avg_ratio)}</td>
        <td><div class="bar"><i class="${barClass(g.avg_ratio)}" style="width:${Math.round((g.avg_ratio || 0) * 100)}%"></i></div></td>
      </tr>`).join('') + '</tbody></table>' : '<div class="empty">Нет данных.</div>';

    $('ovKinds').innerHTML = o.kind_breakdown.length ? `<table><thead><tr><th>Тип</th><th class="num">Кол-во</th><th class="num">Результат</th></tr></thead><tbody>` +
      o.kind_breakdown.map((k) => `<tr>
        <td>${esc(kindLabel(k.kind))}</td>
        <td class="num">${k.count}</td>
        <td class="num">${pct(k.avg_ratio)}</td>
      </tr>`).join('') + '</tbody></table>' : '<div class="empty">Нет данных.</div>';
  } catch (e) {
    toast('Не удалось загрузить обзор: ' + e.message, 'err');
  }
}

function kindLabel(k) {
  return ({
    task: 'Домашние работы',
    quiz: 'Контрольные',
    submission: 'Проекты',
    attendance: 'Посещаемость',
    grade: 'Итоговые оценки',
  })[k] || k;
}

async function seedDemo() {
  if (!confirm('Перезаписать текущие данные демонстрационным набором?')) return;
  try {
    const r = await api('/api/v1/seed', { method: 'POST' });
    toast(`Сгенерировано активностей: ${r.activities}`, 'ok');
    loadOverview();
    loadStudents();
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

// ─────────────────────────────── Ученики ───────────────────────────────

async function loadStudents() {
  const q = encodeURIComponent($('qStudents').value);
  const g = encodeURIComponent($('fGroup').value);
  const s = encodeURIComponent($('fSort').value);
  try {
    const r = await api(`/api/v1/students?q=${q}&group=${g}&sort=${s}`);
    state.students = r.students || [];

    // Группы для фильтра (загружаем один раз и кешируем опции).
    if (!$('fGroup').dataset.loaded) {
      const gr = await api('/api/v1/groups');
      const sel = $('fGroup');
      gr.groups.forEach((x) => {
        const o = document.createElement('option');
        o.value = x; o.textContent = x;
        sel.appendChild(o);
      });
      sel.dataset.loaded = '1';
    }

    if (!state.students.length) {
      $('studentsList').innerHTML = '<div class="empty">Ученики не найдены. Сгенерируйте демо-данные или импортируйте файл.</div>';
    } else {
      $('studentsList').innerHTML = `<div class="list"><table>
        <thead><tr>
          <th>Ученик</th><th>Группа</th><th>Курс</th>
          <th class="num">Активностей</th><th class="num">Результат</th>
          <th class="num">Время</th><th>Последняя</th><th></th>
        </tr></thead><tbody>` +
        state.students.map((s) => `<tr>
          <td><b>${esc(s.full_name)}</b><br><span class="mono">${esc(s.external_id)}</span></td>
          <td>${esc(s.group_name) || '—'}</td>
          <td>${s.grade ?? '—'}</td>
          <td class="num">${s.activities}</td>
          <td class="num">${pct(s.avg_ratio)}</td>
          <td class="num">${s.total_minutes != null ? Math.round(s.total_minutes) + ' мин' : '—'}</td>
          <td class="muted">${esc(fmtDate(s.last_activity))}</td>
          <td style="text-align:right;white-space:nowrap">
            <button class="b b-sm" onclick="openStudent(${s.id})">Открыть</button>
            <button class="b b-sm b-p" onclick="quickAnalyze(${s.id})">ИИ</button>
          </td>
        </tr>`).join('') + '</tbody></table></div>';
    }
    syncStudentSelects();
  } catch (e) {
    toast('Ошибка загрузки учеников: ' + e.message, 'err');
  }
}

function syncStudentSelects() {
  const opts = state.students.map((s) =>
    `<option value="${s.id}">${esc(s.full_name)} (${esc(s.group_name || s.external_id)})</option>`).join('');
  ['anStudent', 'anA', 'anB', 'fqStudent'].forEach((id) => {
    const el = $(id);
    if (!el) return;
    const cur = el.value;
    el.innerHTML = opts;
    if (cur && state.students.some((s) => String(s.id) === cur)) el.value = cur;
  });
  // Для сравнения по умолчанию — разные ученики.
  const a = $('anA'), b = $('anB');
  if (a && b && state.students.length > 1) {
    if (!a.value) a.value = state.students[0].id;
    if (!b.value || b.value === a.value) b.value = state.students[1].id;
  }
}

async function openStudent(id) {
  try {
    const d = await api('/api/v1/students/' + id);
    const s = d.summary;
    state.currentStudent = d;
    $('sdTitle').innerHTML = `${esc(s.full_name)} <span class="muted">${esc(s.external_id)} · ${esc(s.group_name)}</span>`;

    const maxRatio = Math.max(...d.timeline.map((t) => t.count), 1);
    const spark = d.timeline.length ? `<div class="spark">` + d.timeline.map((t) =>
      `<div style="height:${Math.round((t.count / maxRatio) * 100)}%" title="${esc(t.period)}: ${t.count}"><span>${esc(t.period.slice(5))}</span></div>`).join('') + `</div>` : '';

    let html = `<div class="cards">
      <div class="card"><div class="k">Активностей</div><div class="v">${s.activities}</div></div>
      <div class="card"><div class="k">Средний результат</div><div class="v">${pct(s.avg_ratio)}</div></div>
      <div class="card"><div class="k">Время</div><div class="v">${s.total_minutes != null ? Math.round(s.total_minutes / 60) + ' ч' : '—'}</div></div>
      <div class="card"><div class="k">Курс</div><div class="v">${s.grade ?? '—'}</div></div>
    </div>`;

    if (d.strengths.length) {
      html += `<div class="sec"><div class="sec-t">Сильные стороны</div><ul>` +
        d.strengths.map((s) => `<li>${esc(s)}</li>`).join('') + '</ul></div>';
    }
    if (d.weaknesses.length) {
      html += `<div class="sec"><div class="sec-t">Зоны внимания</div><ul>` +
        d.weaknesses.map((s) => `<li>${esc(s)}</li>`).join('') + '</ul></div>';
    }

    if (d.by_subject.length) {
      html += `<div class="sec"><div class="sec-t">По предметам</div><table><thead><tr>
        <th>Предмет</th><th class="num">Работ</th><th class="num">Результат</th><th class="num">Время</th><th style="width:110px"></th>
        </tr></thead><tbody>` + d.by_subject.map((s) => `<tr>
          <td>${esc(s.subject)}</td>
          <td class="num">${s.count}</td>
          <td class="num">${pct(s.avg_ratio)}</td>
          <td class="num">${s.total_minutes != null ? Math.round(s.total_minutes) + ' мин' : '—'}</td>
          <td><div class="bar"><i class="${barClass(s.avg_ratio)}" style="width:${Math.round((s.avg_ratio || 0) * 100)}%"></i></div></td>
        </tr>`).join('') + '</tbody></table></div>';
    }

    if (d.timeline.length) {
      html += `<div class="sec"><div class="sec-t">Динамика по месяцам</div>` + spark + `
        <table style="margin-top:20px"><thead><tr><th>Месяц</th><th class="num">Активностей</th><th class="num">Результат</th></tr></thead><tbody>` +
        d.timeline.map((t) => `<tr><td>${esc(t.period)}</td><td class="num">${t.count}</td><td class="num">${pct(t.avg_ratio)}</td></tr>`).join('') +
        '</tbody></table></div>';
    }

    if (d.recent.length) {
      html += `<div class="sec"><div class="sec-t">Последние работы</div><table><thead><tr>
        <th>Дата</th><th>Работа</th><th>Предмет</th><th class="num">Баллы</th><th>Статус</th>
        </tr></thead><tbody>` + d.recent.slice(0, 30).map((a) => `<tr>
          <td class="muted">${esc(fmtDate(a.occurred_at))}</td>
          <td>${esc(a.title)}</td>
          <td>${esc(a.subject)}</td>
          <td class="num">${a.score != null && a.max_score ? a.score + '/' + a.max_score : '—'}</td>
          <td>${statusTag(a.status)}</td>
        </tr>`).join('') + '</tbody></table></div>';
    }

    html += `<div class="sec" style="display:flex;gap:8px;flex-wrap:wrap">
      <button class="b b-p" onclick="analyzeStudent(${d.id})">Полный разбор (ИИ)</button>
      <button class="b" onclick="askAbout(${d.id})">Задать вопрос</button>
    </div>`;

    $('sdBody').innerHTML = html;
    $('mStudent').classList.add('on');
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

function statusTag(s) {
  const map = {
    done: ['tag-ok', 'выполнено'],
    late: ['tag-w', 'с опозданием'],
    missing: ['tag-r', 'пропущено'],
    submitted: ['tag-ok', 'сдано'],
  };
  const [cls, label] = map[s] || ['tag-a', s || '—'];
  return `<span class="tag ${cls}">${esc(label)}</span>`;
}

function closeStudent() {
  $('mStudent').classList.remove('on');
}

// ─────────────────────────────── Импорт ───────────────────────────────

async function importFile(input) {
  const f = input.files[0];
  if (!f) return;
  const out = $('importResult');
  busy(out, 'Импорт…');
  try {
    const text = await f.text();
    const isJson = f.name.endsWith('.json') || text.trim().startsWith('[');
    const r = await api('/api/v1/import', {
      method: 'POST',
      headers: { 'Content-Type': isJson ? 'application/json' : 'text/csv' },
      body: text,
    });
    out.className = 'output';
    out.innerHTML = `<b>Готово.</b> Строк: ${r.rows_read}, новых учеников: ${r.students_created},
      обновлено: ${r.students_updated}, записей добавлено: ${r.activities_created}.` +
      (r.errors.length ? `<div style="margin-top:8px;color:var(--bad)">Ошибки (${r.errors.length}):<br>` +
        r.errors.slice(0, 15).map((e) => esc(e)).join('<br>') + '</div>' : '');
    loadStudents();
    loadOverview();
    toast('Импорт завершён', 'ok');
  } catch (e) {
    out.className = 'output';
    out.innerHTML = '<span style="color:var(--bad)">' + esc(e.message) + '</span>';
  }
  input.value = '';
}

// ─────────────────────────────── Модели ───────────────────────────────

async function loadModels() {
  try {
    const r = await api('/api/v1/models');
    state.models = r.models || [];
    state.activeModel = r.active || '';

    $('catalogList').innerHTML = state.catalog.length ? state.catalog.map((c) => {
      const file = (r.files || []).find((f) => f[0] === c.id);
      const downloaded = file || state.models.find((m) => m.id === c.id);
      const active = state.activeModel && state.activeModel.includes(c.id);
      return `<div class="list-row">
        <div class="grow">
          <b>${esc(c.name)}</b> <span class="tag">${esc(c.params)}</span>
          ${c.recommended ? '<span class="tag tag-a">рекомендуется</span>' : ''}
          ${active ? '<span class="tag tag-ok">активна</span>' : ''}
          <div class="muted" style="margin-top:3px">${esc(c.description)}</div>
          <div class="muted" style="margin-top:2px">${fmtBytes(c.size_approx)} · нужно RAM: ${c.min_ram_mb} МБ</div>
        </div>
        <div style="text-align:right;white-space:nowrap">
          ${downloaded
            ? `<button class="b b-sm b-g" onclick="activateModel('${esc(c.id)}')">${active ? 'Активна' : 'Сделать активной'}</button>
               <button class="b b-sm b-r" onclick="deleteModel('${esc(c.id)}')">Удалить</button>`
            : `<button class="b b-sm b-p" onclick="downloadModel('${esc(c.id)}')">Скачать</button>`}
        </div>
      </div>`;
    }).join('') : '<div class="empty">Загрузка каталога…</div>';

    const installed = (r.files || []).length ? (r.files || []).map(([id, size]) => {
      const active = state.activeModel && state.activeModel.includes(id);
      return `<div class="list-row">
        <div class="grow"><b>${esc(id)}</b> ${active ? '<span class="tag tag-ok">активна</span>' : ''}
        <div class="muted">${fmtBytes(size)}</div></div>
        <div style="white-space:nowrap">
          <button class="b b-sm b-g" onclick="activateModel('${esc(id)}')">Активировать</button>
          <button class="b b-sm b-r" onclick="deleteModel('${esc(id)}')">Удалить</button>
        </div>
      </div>`;
    }).join('') : '<div class="empty">Моделей пока нет.</div>';

    $('modelList').innerHTML = installed;

    // Прогресс загрузок.
    const dl = r.downloads || [];
    if (dl.length) {
      $('dlStatus').className = 'output';
      $('dlStatus').innerHTML = dl.map((d) => {
        const pctv = d.total ? Math.round((d.received / d.total) * 100) : 0;
        return `<div style="margin-bottom:6px"><b>${esc(d.name)}</b> — ${esc(d.status)}
          ${d.total ? `(${pctv}%, ${fmtBytes(d.received)} / ${fmtBytes(d.total)})` : `(${fmtBytes(d.received)})`}
          ${d.error ? `<br><span style="color:var(--bad)">${esc(d.error)}</span>` : ''}</div>`;
      }).join('');
      // Обновляем, пока идёт загрузка.
      if (dl.some((d) => d.status === 'running')) setTimeout(loadModels, 1200);
    }

    updateActiveLabel();
  } catch (e) {
    toast('Ошибка загрузки моделей: ' + e.message, 'err');
  }
}

async function loadCatalog() {
  try {
    const r = await api('/api/v1/models/catalog');
    state.catalog = r.catalog || [];
    loadModels();
  } catch (e) {
    toast('Ошибка каталога: ' + e.message, 'err');
  }
}

async function downloadModel(id) {
  try {
    await api('/api/v1/models/download', { method: 'POST', body: { id } });
    toast('Загрузка началась', 'ok');
    loadModels();
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

async function downloadCustom() {
  const id = $('cmId').value.trim();
  const name = $('cmName').value.trim() || id;
  const url = $('cmUrl').value.trim();
  if (!id || !url) { toast('Нужны идентификатор и ссылка', 'err'); return; }
  try {
    await api('/api/v1/models/download/custom', { method: 'POST', body: { id, name, url } });
    toast('Загрузка началась', 'ok');
    loadModels();
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

async function activateModel(id) {
  try {
    await api('/api/v1/models/activate', { method: 'POST', body: { id } });
    toast('Модель активирована: ' + id, 'ok');
    loadModels();
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

async function deleteModel(id) {
  if (!confirm('Удалить модель ' + id + '?')) return;
  try {
    await api('/api/v1/models/' + encodeURIComponent(id), { method: 'DELETE' });
    toast('Удалено', 'ok');
    loadModels();
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

// ─────────────────────────────── Параметры ───────────────────────────────

async function loadParams() {
  try {
    const p = await api('/api/v1/params');
    $('pTemp').value = p.temperature;
    $('pTopP').value = p.top_p;
    $('pTopK').value = p.top_k;
    $('pRepeat').value = p.repeat_penalty;
    $('pDry').value = p.dry_multiplier ?? 0.8;
    $('pMaxTok').value = p.max_tokens;
    $('pCtx').value = p.n_ctx;
    $('pThreads').value = p.n_threads;
    $('pSeed').value = p.seed;
    $('pSystem').value = p.system_prompt || '';
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

async function saveParams() {
  const body = {
    temperature: +$('pTemp').value,
    top_p: +$('pTopP').value,
    top_k: +$('pTopK').value,
    repeat_penalty: +$('pRepeat').value,
    dry_multiplier: +$('pDry').value,
    max_tokens: +$('pMaxTok').value,
    n_ctx: +$('pCtx').value,
    n_threads: +$('pThreads').value,
    seed: +$('pSeed').value,
    system_prompt: $('pSystem').value,
  };
  try {
    await api('/api/v1/params', { method: 'POST', body });
    $('paramsMsg').className = 'output';
    $('paramsMsg').innerHTML = '<span style="color:var(--ok)">Сохранено.</span>';
    toast('Параметры сохранены', 'ok');
  } catch (e) {
    toast('Ошибка: ' + e.message, 'err');
  }
}

// ─────────────────────────────── Анализ ───────────────────────────────

function requireModel() {
  if (!state.activeModel) {
    toast('Сначала скачайте и активируйте модель во вкладке «Модели»', 'err');
    tab('models');
    return false;
  }
  return true;
}

async function analyzeStudent(id) {
  if (!requireModel()) return;
  const sid = id || $('anStudent').value;
  if (!sid) { toast('Выберите ученика', 'err'); return; }
  const out = $('anOutput');
  busy(out, 'Модель анализирует данные ученика…');
  try {
    const r = await api('/api/v1/analyze/student', {
      method: 'POST',
      body: { student_id: +sid, focus: $('anFocus').value.trim() || null },
    });
    setOutput(out, r.text, r.loop_detected);
    if (id) { $('mStudent').classList.remove('on'); tab('analyze'); $('anStudent').value = String(sid); }
  } catch (e) {
    out.className = 'output';
    out.innerHTML = '<span style="color:var(--bad)">' + esc(e.message) + '</span>';
  }
}

async function quickAnalyze(id) {
  if (!requireModel()) return;
  $('anStudent').value = String(id);
  tab('analyze');
  await analyzeStudent(id);
}

async function analyzeCompare() {
  if (!requireModel()) return;
  const a = $('anA').value, b = $('anB').value;
  if (!a || !b) { toast('Выберите двух учеников', 'err'); return; }
  if (a === b) { toast('Выберите разных учеников', 'err'); return; }
  const out = $('anOutput');
  busy(out, 'Модель сравнивает учеников…');
  try {
    const r = await api('/api/v1/analyze/compare', { method: 'POST', body: { a: +a, b: +b } });
    setOutput(out, r.text, r.loop_detected);
  } catch (e) {
    out.className = 'output';
    out.innerHTML = '<span style="color:var(--bad)">' + esc(e.message) + '</span>';
  }
}

async function analyzeOverview() {
  if (!requireModel()) return;
  const out = $('ovAnalysis');
  busy(out, 'Модель анализирует поток…');
  try {
    const r = await api('/api/v1/analyze/overview', { method: 'POST', body: {} });
    setOutput(out, r.text, r.loop_detected);
  } catch (e) {
    out.className = 'output';
    out.innerHTML = '<span style="color(--bad)">' + esc(e.message) + '</span>';
  }
}

async function askQuestion() {
  if (!requireModel()) return;
  const sid = $('fqStudent').value;
  const q = $('fqText').value.trim();
  if (!sid || !q) { toast('Нужны ученик и вопрос', 'err'); return; }
  const out = $('fqAnswer');
  busy(out, 'Модель отвечает…');
  try {
    const r = await api('/api/v1/analyze/ask', {
      method: 'POST',
      body: { student_id: +sid, question: q },
    });
    setOutput(out, r.text);
  } catch (e) {
    out.className = 'output';
    out.innerHTML = '<span style="color:var(--bad)">' + esc(e.message) + '</span>';
  }
}

async function askAbout(id) {
  $('fqStudent').value = String(id);
  tab('analyze');
  $('fqText').focus();
}

// ─────────────────────────────── Старт ───────────────────────────────

document.addEventListener('DOMContentLoaded', async () => {
  applyAccent(state.accent);
  setTheme(state.theme);
  loadCatalog();

  $('mStudent').addEventListener('click', (e) => {
    if (e.target === $('mStudent')) closeStudent();
  });

  $('iPass').addEventListener('keydown', (e) => { if (e.key === 'Enter') doLogin(); });
  $('iUser').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('iPass').focus(); });

  // Если сессия ещё жива — сразу в приложение.
  try {
    const me = await api('/api/v1/me');
    if (!me.anonymous || localStorage.getItem('eq-token')) {
      await boot();
      return;
    }
  } catch { /* не авторизован — показываем форму входа */ }

  // Без авторизации сразу показываем приложение.
  try {
    await api('/api/v1/overview');
    $('login').style.display = 'none';
    $('app').style.display = '';
    await loadOverview();
  } catch {
    showLogin();
  }
});

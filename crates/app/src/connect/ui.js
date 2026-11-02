/* Browser preferences stay on this device; playback remains on the host. */
// Never reuse guest selections after the host changes its catalog provider.
(() => {
  const hostFetch = window.fetch.bind(window);
  let provider = null;
  window.fetch = async (input, init = {}) => {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href);
    if (url.origin !== location.origin || !url.pathname.startsWith('/api/')) return hostFetch(input, init);
    const captured = provider;
    const method = String(init.method || 'GET').toUpperCase();
    if (captured && url.pathname !== '/api/state') url.searchParams.set('host_provider', captured);
    if (method === 'POST' && captured) {
      const payload = init.body ? JSON.parse(init.body) : {};
      init = {...init, headers: {...init.headers, 'Content-Type': 'application/json'}, body: JSON.stringify({...payload, provider: captured})};
    }
    const response = await hostFetch(url, init);
    if (url.pathname === '/api/state' && response.ok) {
      const state = await response.clone().json();
      if (provider && state.provider && provider !== state.provider) {
        location.reload();
        throw new Error('Host provider changed');
      }
      provider = state.provider || provider;
    } else if (captured && captured !== provider) {
      throw new Error('Host provider changed');
    }
    return response;
  };
})();
(() => {
  let language = localStorage.getItem('deeznats_language') === 'tr' ? 'tr' : 'en';
  let translations = {};
  const textRecords = new WeakMap();
  const attributeRecords = new WeakMap();
  const contentSelector = '.profile-name,.lyric-line,.lyric-gap,.lyric-translation,.queue-title,.queue-artist,.shelf-title,.shelf-subtitle,.genre-chip,#mini-title,#mini-artist,#modal-title,#lyrics-title,#lyrics-provider,#language-select,.native-icon,script,style';
  function translate(value) {
    if (language !== 'tr') return value;
    const whitespace = value.match(/^(\s*)([\s\S]*?)(\s*)$/);
    const text = whitespace[2];
    let translated = translations[text];
    if (!translated) {
      const prefix = text.match(/^([^\p{L}\p{N}]*)([\s\S]+)$/u);
      if (prefix && translations[prefix[2]]) translated = prefix[1] + translations[prefix[2]];
    }
    if (!translated) {
      const count = text.match(/^(.*?)( \(\d+\))$/);
      if (count && translations[count[1]]) translated = translations[count[1]] + count[2];
      const dynamic = text.match(/^(.*?)(genres ready|genres enabled|unsaved changes)$/);
      if (dynamic && translations[dynamic[2]]) translated = dynamic[1] + translations[dynamic[2]];
      const toast = text.match(/^(Up next:|Added to queue:|Playing:)(.*)$/);
      if (toast && translations[toast[1]]) translated = translations[toast[1]] + toast[2];
      const filter = text.match(/^(".*") has no matching genres\.$/);
      if (filter) translated = filter[1] + ' ile eşleşen tür bulunamadı.';
    }
    return whitespace[1] + (translated || text) + whitespace[3];
  }
  window.tr = translate;
  function englishText(value) {
    const whitespace=value.match(/^(\s*)([\s\S]*?)(\s*)$/);
    const match=Object.entries(translations).find(([,turkish])=>turkish===whitespace[2]);
    return match ? whitespace[1]+match[0]+whitespace[3] : value;
  }
  const glyphs = new Map([
    ['🌘','e708'],['🔊','e767'],['🔉','e767'],['🔇','e74f'],['🎵','e8d6'],['📜','e8fd'],['🧭','e721'],['🔍','e721'],
    ['🎛️','e713'],['🎧','e7f6'],['💾','e74e'],['⚠️','e7ba'],['⏳','e917'],['🔴','e91f'],['✓','e73e'],['✕','e711'],
    ['⏮','e892'],['⏭','e893'],['▶','e768'],['⏸','e769'],['♫','ec71'],['☕','e7c1'],['⚡','e945'],['🔥','e7c1'],['🌊','e8d6'],['🌧️','e753'],['🎯','e81d'],['🎉','e734']
  ]);
  function localizeText(node) {
    const parent = node.parentElement;
    if (!parent || parent.closest(contentSelector)) return;
    let record = textRecords.get(node);
    if (!record || node.nodeValue !== record.rendered) record = { original: englishText(node.nodeValue) };
    const match = [...glyphs].find(([symbol]) => record.original.includes(symbol));
    if (match) {
      const [symbol, glyph] = match;
      const button=parent.closest('button');
      if(button && !button.textContent.replace(symbol,'').trim()) {
        const labels={'e892':'Previous','e893':'Next','e768':'Play','e769':'Pause','ec71':'Lyrics','e711':'Close','e767':'Volume'};
        if(labels[glyph]) button.setAttribute('aria-label',labels[glyph]);
      }
      const parts = record.original.split(symbol);
      const fragment = document.createDocumentFragment();
      parts.forEach((part, index) => {
        if (index) { const icon = document.createElement('i'); icon.className = 'native-icon'; icon.setAttribute('aria-hidden','true'); icon.textContent = String.fromCodePoint(parseInt(glyph,16)); fragment.append(icon); }
        if (part) fragment.append(document.createTextNode(part));
      });
      node.replaceWith(fragment); return;
    }
    record.rendered = translate(record.original);
    textRecords.set(node, record);
    if (node.nodeValue !== record.rendered) node.nodeValue = record.rendered;
  }
  function localizeAttributes(element) {
    if (element.closest(contentSelector)) return;
    let records = attributeRecords.get(element);
    if (!records) { records = {}; attributeRecords.set(element,records); }
    for (const key of ['title','placeholder','aria-label']) {
      if (!element.hasAttribute(key)) continue;
      const current = element.getAttribute(key);
      let record = records[key];
      if (!record || current !== record.rendered) record = { original: englishText(current) };
      record.rendered = translate(record.original); records[key] = record;
      if (record.rendered !== current) element.setAttribute(key,record.rendered);
    }
  }
  function localize(root) {
    if (root.nodeType === Node.TEXT_NODE) { localizeText(root); return; }
    if (root.nodeType !== Node.ELEMENT_NODE || root.closest(contentSelector)) return;
    localizeAttributes(root);
    for (const child of [...root.childNodes]) localize(child);
  }
  function setLanguage(value) {
    language = value === 'tr' ? 'tr' : 'en';
    localStorage.setItem('deeznats_language',language);
    document.documentElement.lang = language;
    document.getElementById('language-select').value = language;
    document.getElementById('language-select').setAttribute('aria-label',translate('Language'));
    localize(document.body);
  }

  let catalog = { themes: [] };
  let appearance = { theme:8, blur:20, opacity:40 };
  let expanded = false;
  let artwork = '';
  function rgba(hex, alpha) {
    const value = parseInt(hex.slice(1),16);
    return `rgba(${value>>16},${(value>>8)&255},${value&255},${alpha})`;
  }
  function channels(hex) {const n=parseInt(hex.slice(1),16);return [n>>16,(n>>8)&255,n&255];}
  function brightness(hex) {const [r,g,b]=channels(hex);return (.2126*r+.7152*g+.0722*b)/255;}
  function blend(a,b,weight) {
    const x=channels(a),y=channels(b);
    return '#'+x.map((v,i)=>Math.round(v*weight+y[i]*(1-weight)).toString(16).padStart(2,'0')).join('');
  }
  function luminance(hex) {const [r,g,b]=channels(hex).map(v=>{v/=255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4});return .2126*r+.7152*g+.0722*b;}
  function contrast(a,b) {const x=luminance(a),y=luminance(b);return (Math.max(x,y)+.05)/(Math.min(x,y)+.05);}
  function readableText(color,surfaces,minimum) {
    const score=value=>Math.min(...surfaces.map(surface=>contrast(value,surface)));
    if(score(color)>=minimum) return color;
    const target=score('#141418')>score('#f8f8f8')?'#141418':'#f8f8f8';
    for(let amount=.05;amount<=1;amount+=.05) {const candidate=blend(target,color,Math.min(1,amount));if(score(candidate)>=minimum)return candidate;}
    return target;
  }
  function saveAppearance() { localStorage.setItem('deeznats_appearance',JSON.stringify(appearance)); }
  function applyAppearance() {
    const selected = catalog.themes.find(theme=>theme.id === appearance.theme) || catalog.themes[0];
    if (!selected) return;
    const palette={...selected.palette};
    const effectiveLight=selected.light;
    const surfaces=[palette.Background,palette.Panel,palette.Raised];
    palette.Text=readableText(palette.Text,surfaces,4.5);
    palette.Muted=readableText(palette.Muted,surfaces,4.5);
    palette.AccentText=contrast('#141418',palette.Accent)>contrast('#ffffff',palette.Accent)?'#141418':'#ffffff';
    const alpha=appearance.opacity/100;
    const root = document.documentElement;
    const values = {
      '--bg':palette.Background,'--bg-card':rgba(palette.Panel,alpha),'--bg-card-hover':rgba(palette.Raised,alpha),
      '--panel-solid':palette.Panel,'--raised-solid':palette.Raised,'--panel-bg':rgba(palette.Panel,alpha),'--raised-bg':rgba(palette.Raised,alpha),'--accent-bg':rgba(palette.Accent,alpha),'--panel-blur':(appearance.blur*.5)+'px','--border':palette.Border,'--border-light':palette.Border,
      '--accent':palette.Accent,'--accent-hover':palette.Accent,'--accent-glow':rgba(palette.Accent,.24),
      '--text-main':palette.Text,'--text-sub':palette.Muted,'--text-muted':palette.Muted,'--accent-text':palette.AccentText || '#ffffff','--ui-text-shadow':effectiveLight?'#ffffffbb':'#000000bb'
    };
    for (const [key,value] of Object.entries(values)) root.style.setProperty(key,value);
    root.style.colorScheme = effectiveLight ? 'light' : 'dark';
    document.body.classList.toggle('light-theme',effectiveLight);
    const backdrop = document.getElementById('song-backdrop');
    backdrop.style.backgroundImage = artwork && !artwork.includes("/images/cover/d41d8cd98f00b204e9800998ecf8427e/") ? `url(${JSON.stringify(artwork)})` : 'none';
    const backpanel=document.getElementById('theme-backpanel');
    backpanel.style.backgroundColor=rgba(palette.Background,appearance.opacity/100);
    backpanel.style.backdropFilter=`blur(${appearance.blur*.5}px)`;
    backpanel.style.webkitBackdropFilter=backpanel.style.backdropFilter;
    for(const key of ['blur','opacity']) {
      document.getElementById('appearance-'+key).value=appearance[key];
      document.getElementById('appearance-'+key+'-value').textContent=appearance[key]+'%';
    }
    for(const button of document.querySelectorAll('[data-theme-id]')) button.setAttribute('aria-pressed',String(Number(button.dataset.themeId)===appearance.theme));
  }
  function renderThemes() {
    const container = document.getElementById('appearance-themes'); container.replaceChildren();
    const sorted = [...catalog.themes];
    for (const theme of expanded ? sorted : sorted.slice(0,4)) {
      const button = document.createElement('button');button.className='theme-choice';button.dataset.themeId=theme.id;
      const preview=document.createElement('span');preview.className='theme-preview';preview.style.background=theme.palette.Background;
      const panel=document.createElement('span');panel.style.background=theme.palette.Panel;
      const accent=document.createElement('span');accent.style.background=theme.palette.Accent;
      preview.append(panel,accent);
      const name=document.createElement('span');name.className='theme-name';name.textContent=theme.name;
      button.append(preview,name);button.onclick=()=>setAppearance('theme',theme.id);container.append(button);
    }
    document.getElementById('appearance-more').textContent=expanded?'See less':'See more';
  }
  function setAppearance(key,value) {
    if(key==='theme') {
      if(!catalog.themes.some(theme=>theme.id===Number(value))) return;
      appearance.theme=Number(value);
    } else if(key==='blur' || key==='opacity') appearance[key]=Math.max(0,Math.min(100,Number(value)||0));
    else return;
    saveAppearance();applyAppearance();
  }
  function songChanged(data) {
    const value=data.artwork || '';
    if(value===artwork) return;
    artwork=value;applyAppearance();
  }
  async function init() {
    const hostInput=document.getElementById('host-address');
    hostInput.value=window.location.origin;
    document.getElementById('host-address-form').addEventListener('submit',event=>{
      event.preventDefault();
      try {
        const raw=hostInput.value.trim();
        const host=new URL(raw.includes('://')?raw:'http://'+raw);
        if(!['http:','https:'].includes(host.protocol)||host.username||host.password||host.pathname!=='/'||host.search||host.hash) throw new Error();
        window.location.assign(host.origin);
      } catch(error) {
        document.getElementById('host-address-status').textContent=translate('Enter only the host address and port');
      }
    });
    const backdrop=document.createElement('div');backdrop.id='song-backdrop';backdrop.setAttribute('aria-hidden','true');
    const backpanel=document.createElement('div');backpanel.id='theme-backpanel';backpanel.setAttribute('aria-hidden','true');
    document.body.prepend(backpanel);document.body.prepend(backdrop);
    setLanguage(language);
    new MutationObserver(mutations=>{
      for(const mutation of mutations) {
        if(mutation.type==='characterData') localizeText(mutation.target);
        else if(mutation.type==='attributes') localizeAttributes(mutation.target);
        else for(const node of [...mutation.addedNodes]) localize(node);
      }
    }).observe(document.body,{subtree:true,childList:true,characterData:true,attributes:true,attributeFilter:['title','placeholder','aria-label']});
    try { const response=await fetch('/translations.json');translations=await response.json();setLanguage(language); } catch(error) {}
    try {
      const response=await fetch('/api/appearance');if(!response.ok) throw new Error('Appearance unavailable');catalog=await response.json();
      let saved={};try {saved=JSON.parse(localStorage.getItem('deeznats_appearance') || '{}')} catch(error) {}
      const defaults=catalog.defaults || {};
      appearance={theme:Number(saved.theme ?? defaults.theme ?? 8),blur:Number(saved.blur ?? defaults.blur ?? 20),opacity:Number(saved.opacity ?? saved.tint ?? defaults.opacity ?? defaults.tint ?? 40)};
      if(!catalog.themes.some(theme=>theme.id===appearance.theme)) appearance.theme=8;
      for(const key of ['blur','opacity']) appearance[key]=Math.max(0,Math.min(100,Number(appearance[key])||0));
      saveAppearance();renderThemes();applyAppearance();
    } catch(error) {document.getElementById('appearance-status').textContent='Could not load appearance settings.';}
  }
  window.JamUI={setLanguage,songChanged,appearance:setAppearance,expandThemes(){expanded=!expanded;renderThemes();applyAppearance()}};
  document.addEventListener('DOMContentLoaded',init);
})();

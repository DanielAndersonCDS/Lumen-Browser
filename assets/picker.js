(()=>{
  if(window.__lumenPick)return;window.__lumenPick=1;
  let last;
  const st=document.createElement('style');
  st.textContent='.__lh{outline:2px solid #2563eb!important;background:rgba(37,99,235,.10)!important;cursor:crosshair!important}';
  document.documentElement.appendChild(st);
  const sel=el=>{
    if(el.id)return '#'+CSS.escape(el.id);
    const parts=[];
    while(el&&el.nodeType===1&&el!==document.body){
      let s=el.tagName.toLowerCase();
      const c=[...el.classList].filter(x=>x!=='__lh'&&!/\d{3,}/.test(x)).slice(0,2);
      if(c.length)s+='.'+c.map(CSS.escape).join('.');
      const p=el.parentElement;
      if(p&&!c.length){const same=[...p.children].filter(x=>x.tagName===el.tagName);if(same.length>1)s+=':nth-of-type('+(same.indexOf(el)+1)+')'}
      parts.unshift(s);
      if(el.id||parts.length>=3)break;
      el=el.parentElement;
    }
    return parts.join('>');
  };
  const mv=e=>{last&&last.classList.remove('__lh');last=e.target;last.classList.add('__lh')};
  const done=()=>{
    removeEventListener('mouseover',mv,true);removeEventListener('click',ck,true);removeEventListener('keydown',kd,true);
    last&&last.classList.remove('__lh');st.remove();window.__lumenPick=0;
  };
  const ck=e=>{
    if(!e.isTrusted)return;
    e.preventDefault();e.stopPropagation();
    const t=e.target;t.classList.remove('__lh');
    const s=sel(t);done();t.style.display='none';
    window.ipc.postMessage(JSON.stringify({cmd:'hide-save',sel:s}));
  };
  const kd=e=>{if(e.key==='Escape'){e.preventDefault();done()}};
  addEventListener('mouseover',mv,true);addEventListener('click',ck,true);addEventListener('keydown',kd,true);
})();

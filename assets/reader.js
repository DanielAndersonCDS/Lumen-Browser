(()=>{
  const root=document.documentElement;
  if(root.dataset.lumenReader){location.reload();return}
  const score=el=>[...el.children].reduce((s,c)=>s+(c.tagName==='P'?c.textContent.trim().length:0),0);
  let best=document.querySelector('article'),bs=best?score(best):0;
  document.querySelectorAll('main,section,div').forEach(el=>{const s=score(el);if(s>bs){bs=s;best=el}});
  if(!best||bs<200)best=document.querySelector('article')||document.querySelector('main')||document.body;
  const clone=best.cloneNode(true);
  clone.querySelectorAll('script,style,nav,aside,footer,form,iframe,noscript,button,svg,[role=banner],[role=navigation],[aria-hidden=true]').forEach(n=>n.remove());
  const h1=document.querySelector('h1');
  const title=h1?h1.textContent.trim():document.title;
  const css='html{background:#fff}body{margin:0;padding:48px 20px;color:#1f2430;font:18px/1.75 Georgia,"Times New Roman",serif}main{max-width:680px;margin:0 auto}h1{font:700 30px/1.3 system-ui,sans-serif;margin:0 0 24px}img,video{max-width:100%;height:auto}a{color:#2563eb}pre,code{font-family:"JetBrains Mono",Consolas,monospace;font-size:14px;background:#f5f6f8}pre{padding:12px;overflow:auto}blockquote{border-left:3px solid #e4e7ec;margin:0;padding-left:16px;color:#6b7280}';
  const m=document.createElement('main');
  const hh=document.createElement('h1');hh.textContent=title;m.appendChild(hh);m.appendChild(clone);
  document.head.innerHTML='<meta charset="utf-8"><title></title><style>'+css+'</style>';
  document.title=title;
  document.body.removeAttribute('class');document.body.removeAttribute('style');
  document.body.replaceChildren(m);
  root.dataset.lumenReader='1';scrollTo(0,0);
})();

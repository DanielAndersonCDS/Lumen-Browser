(()=>{
  const H=__HOSTS__;
  const bad=u=>{try{const h=new URL(u,location.href).hostname;return H.some(d=>h===d||h.endsWith('.'+d))}catch(e){return false}};
  const of=window.fetch;
  if(of)window.fetch=function(i){const u=typeof i==='string'?i:(i&&i.url);return bad(u)?Promise.resolve(new Response('',{status:204})):of.apply(this,arguments)};
  const ox=XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open=function(m,u){if(bad(u))arguments[1]='data:,';return ox.apply(this,arguments)};
  const sb=navigator.sendBeacon;
  if(sb)navigator.sendBeacon=function(u){return bad(u)?true:sb.apply(navigator,arguments)};
  for(const [C,p] of [[HTMLScriptElement,'src'],[HTMLImageElement,'src'],[HTMLIFrameElement,'src'],[HTMLLinkElement,'href']]){
    const d=Object.getOwnPropertyDescriptor(C.prototype,p);
    if(d&&d.set)Object.defineProperty(C.prototype,p,{configurable:true,enumerable:d.enumerable,get:d.get,set(v){d.set.call(this,bad(v)?'data:,':v)}});
  }
  const sa=Element.prototype.setAttribute;
  Element.prototype.setAttribute=function(n,v){
    if((n==='src'||n==='href')&&/^(SCRIPT|IMG|IFRAME|LINK)$/.test(this.tagName)&&bad(v))v='data:,';
    return sa.call(this,n,v);
  };
  const css='ins.adsbygoogle,.adsbygoogle,[id^="google_ads"],[id^="div-gpt-ad"],[class*="ad-slot"],[class*="adslot"],[id*="ad-slot"],[aria-label="Advertisement"],iframe[src*="doubleclick.net"],iframe[src*="googlesyndication.com"],.taboola,.trc_related_container,.OUTBRAIN,[id^="taboola-"],[id^="outbrain_"]{display:none!important}';
  const add=()=>{const s=document.createElement('style');s.textContent=css;(document.head||document.documentElement).appendChild(s)};
  if(document.documentElement)add();else new MutationObserver((_,o)=>{if(document.documentElement){o.disconnect();add()}}).observe(document,{childList:true});
  new MutationObserver(ms=>{for(const m of ms)for(const n of m.addedNodes){if(n.src&&bad(n.src))n.remove()}}).observe(document,{childList:true,subtree:true});
})();

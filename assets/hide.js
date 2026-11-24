(()=>{
  const M=__HIDE__;
  const s=M[location.hostname];
  if(!s||!s.length)return;
  const add=()=>{const st=document.createElement('style');st.textContent=s.join(',')+'{display:none!important}';(document.head||document.documentElement).appendChild(st)};
  if(document.documentElement)add();else new MutationObserver((_,o)=>{if(document.documentElement){o.disconnect();add()}}).observe(document,{childList:true});
})();

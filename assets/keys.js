(()=>{
  if(window.__lumenKeys)return;window.__lumenKeys=1;
  const N='__NONCE__',J=JSON.stringify.bind(JSON);let P=window.ipc&&window.ipc.postMessage.bind(window.ipc);
  const send=o=>{try{P=P||(window.ipc&&window.ipc.postMessage.bind(window.ipc));o.n=N;P&&P(J(o))}catch(e){}};
  addEventListener('keydown',e=>{
    if(!e.isTrusted||!e.ctrlKey||e.altKey||e.metaKey)return;
    const k=e.key.toLowerCase();
    let c=null;
    if(e.shiftKey){
      if(k==='t')c='reopen';else if(k==='n')c='private';else if(k==='r')c='reader';
      else if(k==='h')c='hide';else if(k==='u')c='unhide';else if(k==='b')c='bookmarks';
      else if(k==='tab')c='prev';
    }else{
      if(k==='l')c='address';else if(k==='t')c='new';else if(k==='w')c='close';
      else if(k==='d')c='bookmark';else if(k==='y')c='history';else if(k==='b')c='fold';
      else if(k==='tab')c='next';else if(k===',')c='settings';else if(k>='1'&&k<='9')c='tab'+k;
    }
    if(!c)return;
    e.preventDefault();e.stopPropagation();
    send({cmd:'key',combo:c});
  },true);
})();

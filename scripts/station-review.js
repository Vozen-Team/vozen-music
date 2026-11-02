const $ = (id) => document.getElementById(id);
let tracks = [], current = 0, reviews = {};
try { reviews = JSON.parse(localStorage.getItem('vozen-music-listening-review-v1') || '{}'); } catch { $('notice').textContent = 'Browser storage is unavailable. Export your review before closing.'; }
const key = (t) => t.technical_review?.sha256 || t.file;
function visible() { return tracks.map((t, i) => ({ t, i })).filter(({t}) => t.station === $('station').value); }
function drawList() {
 const items = visible(); $('count').textContent = `${$('station').selectedOptions[0].text}: ${items.length} recordings`;
 $('list').replaceChildren(...items.map(({t,i}) => { const b=document.createElement('button'); b.textContent=t.title; b.setAttribute('aria-current',String(i===current)); b.onclick=()=>show(i); return b; }));
}
function show(i) {
 current=i; const t=tracks[i], q=t.technical_review, r=reviews[key(t)] || {};
 $('title').textContent=t.title; $('credit').textContent=t.artist+' · '+t.license; $('source').href=t.source_url; $('rain-source').hidden=!t.rain_source; if(t.rain_source) $('rain-source').href=t.rain_source.source_url;
 $('details').textContent=q ? `${Math.floor(q.duration_seconds/60)}:${String(Math.floor(q.duration_seconds%60)).padStart(2,'0')} · ${q.sample_rate} Hz · Full decode passed · gain for bot: ${q.gain_db} dB. ${t.mix_recipe ? 'Fixed offline music + rain mix.' : 'Original recording.'}` : 'Technical review pending.';
 $('rain-note').textContent=t.station==='rain' ? (t.mix_recipe ? 'Rain is integrated in this file. Both sources declare CC0; full human listening remains pending.' : 'Rain preparation pending. This source is not yet approved as music with integrated rain.') : '';
 $('player').pause(); $('player').src=t.file;
 $('player').volume=q ? Math.min(1,10**(q.gain_db/20)) : 1;
 $('complete').checked=!!r.full_recording; $('voice').value=r.voice || ''; $('fit').value=r.fit || ''; $('notes').value=r.notes || ''; drawList();
}
function save() {
 const t=tracks[current]; reviews[key(t)]={file:t.file,title:t.title,sha256:t.technical_review?.sha256,full_recording:$('complete').checked,voice:$('voice').value,fit:$('fit').value,notes:$('notes').value,reviewed_at:new Date().toISOString()};
 try {localStorage.setItem('vozen-music-listening-review-v1',JSON.stringify(reviews));$('notice').textContent='Review saved in this browser.';} catch {$('notice').textContent='Could not save in this browser. Export the review before closing.';}
}
for (const id of ['complete','voice','fit','notes']) $(id).addEventListener('change',save);
$('station').onchange=()=>{const items=visible();if(items.length)show(items[0].i);};
for(const [id,step] of [['previous',-1],['next',1]]) $(id).onclick=()=>{const items=visible(),pos=items.findIndex(({i})=>i===current),next=Math.min(items.length-1,Math.max(0,pos+step));show(items[next].i);};
$('export').onclick=()=>{const blob=new Blob([JSON.stringify({scope:'Vozen Music proposed stations',exported_at:new Date().toISOString(),reviews:Object.values(reviews)},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob),link=document.createElement('a');link.href=url;link.download='vozen-music-listening-review.json';link.click();URL.revokeObjectURL(url);};
fetch('manifest.json').then(r=>{if(!r.ok)throw Error('Manifest unavailable');return r.json();}).then(data=>{tracks=data.tracks;const stations=[...new Set(tracks.map(t=>t.station))];$('station').replaceChildren(...stations.map(id=>{const o=document.createElement('option');o.value=id;o.textContent=id.replaceAll('-',' ').replace(/\b\w/g,c=>c.toUpperCase());return o;}));if(tracks.length)show(visible()[0].i);else $('notice').textContent='No listening candidates are prepared yet.';}).catch(error=>{$('notice').textContent=error.message;});

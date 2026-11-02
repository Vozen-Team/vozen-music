import assert from 'node:assert/strict';
import fs from 'node:fs';
const {chromium}=await import(process.env.VOZEN_PLAYWRIGHT_MODULE||'playwright');
const browser=await chromium.launch();
const base=process.env.VOZEN_PREVIEW_URL||'http://localhost:4783';
const catalog=JSON.parse(fs.readFileSync('catalog.json','utf8')).stations;
try{
 for(const width of [390,1440]){
  const page=await browser.newPage({viewport:{width,height:1000},hasTouch:width===390,isMobile:width===390});
  const mutations=[],requests=[],errors=[];
  page.on('request',r=>{if(!['GET','HEAD'].includes(r.method()))mutations.push(r.url());if(r.url().includes('/recordings/'))requests.push(r.url());});
  page.on('pageerror',e=>errors.push(e.message));
  await page.goto(base+'/music/dashboard?server=1&page=stations&album=lofi');await page.locator('.track-preview').first().waitFor();
  assert.equal(requests.length,0,'No audio downloads before Play');
  assert.equal(await page.locator('.station-tracks a').count(),0,'No inline credit links');
  const first=page.locator('.track-preview').nth(0),second=page.locator('.track-preview').nth(1);
  if(width===1440){
   assert.equal(await first.locator('.preview-number').evaluate(e=>getComputedStyle(e).opacity),'1');
   await page.locator('.station-tracks tbody tr').first().hover();
   await page.waitForFunction(()=>getComputedStyle(document.querySelector('.track-preview .preview-play')).opacity==='1');
  }else assert.equal(await first.locator('.preview-play').evaluate(e=>getComputedStyle(e).opacity),'1','Touch users can find Play');
  await first.click();await page.waitForFunction(()=>{const a=document.querySelector('#editor audio');return a&&!a.paused&&a.currentTime>0.1;});
  assert.equal(await first.getAttribute('aria-pressed'),'true');assert.match(await first.getAttribute('aria-label'),/^Pause preview/);
  assert.equal(await page.locator('#editor audio').count(),1);
  await first.click();assert.equal(await page.locator('#editor audio').evaluate(a=>a.paused),true);
  await first.focus();await first.press('Enter');await page.waitForFunction(()=>!document.querySelector('#editor audio').paused);
  await second.click();await page.waitForFunction(id=>{const a=document.querySelector('#editor audio');return a.src.includes(id)&&!a.paused&&a.currentTime>0;},catalog[0].tracks[1].id);
  assert.equal(await first.getAttribute('aria-pressed'),'false');assert.equal(await second.getAttribute('aria-pressed'),'true');
  if(width===1440){await page.locator('.station-tracks tbody tr').nth(2).hover();await page.screenshot({path:'output/browser-preview-desktop.png',fullPage:true});}
  if(width===390)await page.screenshot({path:'output/browser-preview-mobile.png',fullPage:true});
  const playback = page.locator('#music-pages').getByRole('button',{name:'Playback',exact:true});
  if(!await playback.isVisible())await page.locator('#sections-trigger').click();
  await playback.click();
  assert.equal(await page.locator('#editor audio').count(),0,'Leaving Stations removes the audio player');
  await page.getByRole('heading',{name:'Playback',exact:true}).waitFor();assert.deepEqual(mutations,[]);assert.deepEqual(errors,[]);await page.close();
 }
 const page=await browser.newPage();
 for(const extension of ['ogg','mp3','wav','flac']){
  const station=catalog.find(s=>s.tracks.some(t=>t.file.endsWith('.'+extension)));
  const index=station.tracks.findIndex(t=>t.file.endsWith('.'+extension));
  await page.goto(`${base}/music/dashboard?server=1&page=stations&album=${station.id}`);await page.locator('.track-preview').nth(index).click();
  await page.waitForFunction(()=>{const a=document.querySelector('#editor audio');return a&&!a.paused&&a.readyState>=2&&a.currentTime>0.1;});
  console.log('Decoded browser audio',extension);
 }
 await page.goto(base+'/music/dashboard?server=1&page=stations&album=lofi');
 const audioPath=`**/music/api/recordings/${catalog[0].tracks[0].id}/audio`;
 await page.route(audioPath,route=>route.fulfill({status:502,contentType:'text/html',body:'Temporary error'}));
 await page.locator('.track-preview').first().click();await page.getByText('This track could not be previewed. Reload or try another track.',{exact:true}).waitFor();
 assert.equal(await page.locator('.track-preview').first().getAttribute('aria-pressed'),'false');
 await page.unroute(audioPath);await page.locator('.track-preview').first().click();
 await page.waitForFunction(()=>{const a=document.querySelector('#editor audio');return !a.paused&&a.currentTime>0.1;});
 await page.close();console.log('Browser previews: hover, touch, play/pause/resume/switch, keyboard, stop on navigation, four actual codecs, failure/retry and no Discord writes passed.');
}finally{await browser.close();}

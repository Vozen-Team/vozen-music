import assert from 'node:assert/strict';
import fs from 'node:fs';
const {chromium}=await import(process.env.VOZEN_PLAYWRIGHT_MODULE||'playwright');
const browser=await chromium.launch();
const base=process.env.VOZEN_PREVIEW_URL||'http://localhost:4783';
const stations=JSON.parse(fs.readFileSync('catalog.json','utf8')).stations;
const durations=JSON.parse(fs.readFileSync('web/track-durations.json','utf8'));
const proposals=JSON.parse(fs.readFileSync('web/station-proposals.json','utf8')).filter(proposal=>!stations.some(station=>station.id===proposal.id));
try{
 for(const width of [320,360,390,430,768,900,1440]){
  const page=await browser.newPage({viewport:{width,height:width===1440?1300:1000}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(base+'/music/dashboard?server=1&page=stations');
  await page.locator('.station-albums').waitFor();
  assert.equal(await page.locator('.station-album').count(),stations.length+proposals.length+1);
  await page.locator('.station-album img').evaluateAll(images=>Promise.all(images.map(img=>img.decode())));
  assert.equal(await page.locator('.station-album img').evaluateAll(images=>images.every(i=>i.naturalWidth===640)),true);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`shelf overflow ${width}`);
  await page.locator('#editor').evaluate(e=>Promise.all(e.getAnimations().map(a=>a.finished)));
  if([390,1440].includes(width))await page.screenshot({path:`output/station-albums-${width}.png`,fullPage:true});
  const album=page.locator('.station-album').filter({hasText:'Vozen Rain'});
  await album.focus();await album.press('Enter');await page.locator('.station-tracks').waitFor();
  assert.equal(new URL(page.url()).searchParams.get('album'),'rain');
  assert.equal(new URL(page.url()).searchParams.get('server'),'1');
  assert.equal(await page.locator('.station-tracks tbody tr').count(),25);
  const first=stations.find(s=>s.id==='rain').tracks[0];
  assert.equal(await page.locator('.station-tracks tbody tr').first().locator('.track-text strong').innerText(),first.title);
  assert.equal(await page.locator('.station-tracks tbody tr').first().locator('.track-duration').innerText(),`${Math.floor(durations[first.id]/60)}:${String(Math.floor(durations[first.id]%60)).padStart(2,'0')}`);
  assert.equal(await page.locator('.station-tracks a').count(),0);
  assert.equal(await page.locator('.album-credits a').first().getAttribute('href'),first.source_url);
  assert.equal(await page.locator('.album-credits a').count(),25);
  assert.equal(await page.locator('.track-preview').count(),25);
  assert.equal(await page.getByRole('button',{name:'← Choose another server',exact:true}).count(),0);
  assert.equal(await page.locator('#editor h1').innerText(),'Vozen Rain');
  assert.equal(await page.getByRole('link',{name:'All stations',exact:true}).count(),1);
  assert.equal(await page.locator('.station-album-header .album-back').count(),1);
  assert.equal(await page.locator('.station-album-header').evaluate(e=>getComputedStyle(e.closest('main')).paddingTop),'0px');
  const geometry=await page.locator('.station-album-header').evaluate(hero=>{
   const cover=hero.querySelector('img').getBoundingClientRect(),banner=hero.getBoundingClientRect();
   const cell=document.querySelector('.track-number:has(.track-preview)').getBoundingClientRect(),play=document.querySelector('.track-preview').getBoundingClientRect();
   return {imageWidth:cover.width,bannerWidth:banner.width,imageHeight:cover.height,bannerHeight:banner.height,leftGap:play.left-cell.left,topGap:play.top-cell.top,gradient:getComputedStyle(hero,'::after').backgroundImage};
  });
  assert.equal(geometry.imageWidth,geometry.bannerWidth);assert.equal(geometry.imageHeight,geometry.bannerHeight);
  assert.ok(geometry.leftGap>=7.9&&geometry.topGap>=5.9,'Play has space within the highlighted row');
  assert.match(geometry.gradient,/linear-gradient/);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`track overflow ${width}`);
  if([390,1440].includes(width)){
   await page.locator('.track-identity img').evaluateAll(images=>Promise.all(images.map(i=>{i.loading='eager';return i.decode();})));
   await page.locator('#editor').evaluate(e=>Promise.all(e.getAnimations().map(a=>a.finished)));
   await page.screenshot({path:`output/station-rain-${width}.png`,fullPage:true});
  }
  await page.reload();await page.locator('.station-tracks tbody tr').first().waitFor();
  await page.getByRole('link',{name:'All stations',exact:true}).click();await page.locator('.station-albums').waitFor();
  await page.locator('.station-album').filter({hasText:'My Station'}).click();await page.getByText(/My Station has no fixed tracklist/).waitFor();
  assert.equal(await page.locator('.station-tracks').count(),0);
  await page.getByRole('heading',{name:'Only from Music’s approved catalog',exact:true}).waitFor();
  assert.match(await page.locator('#editor').innerText(),/Uploads, external links and tracks outside the catalog are not accepted/);
  await page.goto(base+'/music/dashboard?server=1&page=stations&album=drum-and-bass');
  await page.locator('.station-tracks').waitFor();
  assert.equal(await page.locator('.track-preview').count(),stations.find(station=>station.id==='drum-and-bass').tracks.length);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`Drum & Bass overflow ${width}`);
  assert.deepEqual(errors,[]);await page.close();
 }
 const page=await browser.newPage();
 for(const proposed of proposals){
  await page.goto(`${base}/music/dashboard?server=1&page=stations&album=${proposed.id}`);
  await page.getByRole('heading',{name:'Music selection in progress',exact:true}).waitFor();
  assert.equal(await page.locator('.station-tracks,audio').count(),0);
  assert.match(await page.locator('#editor').innerText(),/Not available for playback/);
  await page.getByRole('link',{name:'All stations',exact:true}).click();
  await page.locator('.station-albums').waitFor();
 }
 for(const station of stations){
  await page.goto(`${base}/music/dashboard?server=1&page=stations&album=${station.id}`);await page.locator('.station-tracks').waitFor();
  assert.deepEqual(await page.locator('.track-text strong').allTextContents(),station.tracks.map(t=>t.title));
  assert.equal(await page.locator('.album-credits a').count(),station.tracks.length);
 }
 await page.goto(base+'/music/dashboard?server=1&page=stations&album=unknown');await page.locator('.station-albums').waitFor();
 await page.close();console.log('Station albums, recordings, credits, durations, navigation and seven viewport widths passed.');
}finally{await browser.close();}

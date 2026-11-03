import assert from 'node:assert/strict';
const {chromium}=await import(process.env.VOZEN_PLAYWRIGHT_MODULE||'playwright');
const base=process.env.VOZEN_PREVIEW_URL||'http://localhost:4791';
const browser=await chromium.launch();
try {
 for(const width of [390,1440]) for(const phase of ['me','guilds','guilds/1/server-settings']) {
  const page=await browser.newPage({viewport:{width,height:1000}});
  let release;const pending=new Promise(resolve=>{release=resolve;});
  await page.route('**/music/api/'+phase,async route=>{await pending;await route.continue();});
  try {
   await page.goto(base+'/music/dashboard?server=1&page=stations',{waitUntil:'domcontentloaded'});
   await page.locator('#loading:not([hidden]), #workspace-loading:not([hidden])').waitFor();
   await page.screenshot({path:`output/workspace-loading-${width}-${phase.replaceAll('/','-')}.png`});
   assert.equal(await page.locator('#loading .vp-initial-skeleton').isVisible(),false,`Server picker flashed during ${phase}`);
   assert.equal(await page.locator('#sidebar').isVisible(),true);
   assert.equal(await page.locator('.music-workspace-loading').isVisible(),true);
   assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
   release();await page.locator('.station-albums').waitFor();
   let documents=0;page.on('request',r=>{if(r.isNavigationRequest()&&r.frame()===page.mainFrame())documents++;});
   await page.locator('.station-album').filter({hasText:'Vozen Chiptune'}).click();
   await page.getByRole('heading',{name:'Vozen Chiptune',exact:true}).first().waitFor();
   assert.equal(documents,0,'Opening an album reloaded the dashboard');
   await page.getByRole('link',{name:'All stations',exact:true}).click();await page.locator('.station-albums').waitFor();
   await page.goBack();await page.getByRole('heading',{name:'Vozen Chiptune',exact:true}).first().waitFor();
   await page.goBack();await page.locator('.station-albums').waitFor();
   assert.equal(documents,0,'Album back/forward reloaded the dashboard');
   console.log(`Workspace ${width}px ${phase}: no picker flash; album navigation stays in the workspace`);
  } finally {release();await page.close();}
 }
 const firstPaint=await browser.newPage({javaScriptEnabled:false});
 await firstPaint.goto(base+'/music/dashboard?server=1&page=stations&album=rain');
 assert.equal(await firstPaint.locator('#loading').isVisible(),false);
 assert.equal(await firstPaint.locator('#workspace-loading').isVisible(),true);
 assert.equal(await firstPaint.locator('#sidebar').isVisible(),true);
 await firstPaint.close();
 const retry=await browser.newPage();let fail=true;
 await retry.route('**/music/api/guilds/1/server-settings',r=>fail?r.fulfill({status:503,json:{error:'Synthetic settings failure'}}):r.continue());
 await retry.goto(base+'/music/dashboard?server=1&page=stations');
 await retry.locator('#load-retry').waitFor();
 assert.equal(await retry.locator('#loading').isVisible(),false);
 assert.equal(await retry.locator('#workspace-loading').isVisible(),false);
 assert.equal(await retry.locator('#sidebar').isVisible(),true);
 fail=false;await retry.locator('#load-retry').click();await retry.locator('.station-albums').waitFor();
 assert.equal(await retry.locator('#server-picker').isVisible(),false);
 await retry.close();
 console.log('Server-specific first paint without scripts and recoverable settings failure passed');
} finally {await browser.close();}

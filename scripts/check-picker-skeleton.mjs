import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const {chromium}=await import(process.env.VOZEN_PLAYWRIGHT_MODULE||'playwright');
const browser=await chromium.launch();
const base=process.env.VOZEN_PREVIEW_URL||'http://localhost:4783';
try{
  for(const width of [320,768,1440,1910])for(const phase of ['me','guilds']){
    const page=await browser.newPage({viewport:{width,height:1000}});
    let release;const pending=new Promise(resolve=>{release=resolve;});
    await page.route('**/music/api/'+phase,async route=>{await pending;return route.fulfill({json:phase==='me'?{csrf:'fixture',user:{id:'1',username:'QA'}}:{guilds:[{id:'1',name:'Community'}]}});});
    await page.route('**/music/assets/fonts/unbounded-500-*.woff2',async route=>route.fulfill({contentType:'font/woff2',body:await readFile('assets/fonts/'+new URL(route.request().url()).pathname.split('/').pop())}));
    try{
      await page.goto(base+'/music/dashboard');
      const loading=page.locator('#loading');await loading.waitFor({state:'visible'});
      assert.equal(await loading.getAttribute('aria-busy'),'true');
      assert.equal(await loading.locator('.vp-server').count(),4);
      assert.equal(await loading.locator('button,a,input').count(),0);
      await page.evaluate(()=>document.fonts.ready);
      const before=await loading.locator('.vp-server').first().boundingBox();
      await page.screenshot({path:`output/skeleton-music-${phase}-${width}.png`,fullPage:true});
      release();await page.locator('#server-picker').waitFor({state:'visible'});
      assert.equal(await loading.isVisible(),false);
      const row=page.locator('#server-picker .vp-server').first();await row.evaluate(el=>Promise.all(el.getAnimations().map(animation=>animation.finished)));
      const after=await row.boundingBox();for(const key of ['x','y','width','height'])assert.ok(Math.abs(before[key]-after[key])<1,key);
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
      console.log(`Music ${width}px ${phase}: matching skeleton, no interactive placeholders and stable transition passed`);
    }finally{release();await page.close();}
  }
  const page=await browser.newPage({reducedMotion:'reduce'});
  await page.route('**/music/api/me',route=>new Promise(()=>{}));
  await page.goto(base+'/music/dashboard');
  assert.equal(await page.locator('.vp-pulse').first().evaluate(el=>getComputedStyle(el,'::after').animationName),'none');
  await page.close();
}finally{await browser.close();}

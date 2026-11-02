import assert from 'node:assert/strict';
import {mkdir,readFile} from 'node:fs/promises';
const {chromium}=await import(process.env.VOZEN_PLAYWRIGHT_MODULE||'playwright');
const browser=await chromium.launch();
try{
  await mkdir('output',{recursive:true});
  for(const width of [320,768,1440,1910]){
    const page=await browser.newPage({viewport:{width,height:1000}});
    const errors=[];page.on('pageerror',error=>errors.push(error.message));
    await page.route('**/music/api/guilds',route=>route.fulfill({json:{guilds:[{id:'1',name:'Community <safe>'},{id:'2',name:'A long server name that must wrap without overflowing'}]}}));
    if(process.env.SITE_PUBLIC_ROOT)await page.route('**/assets/fonts/*.woff2',async route=>route.fulfill({contentType:'font/woff2',body:await readFile(process.env.SITE_PUBLIC_ROOT+'/assets/fonts/'+new URL(route.request().url()).pathname.split('/').pop())}));
    await page.goto('http://localhost:4783/music/dashboard');
    const picker=page.locator('#server-picker');await picker.waitFor({state:'visible'});
    await page.evaluate(()=>document.fonts.ready);
    await page.locator('.vp-server').evaluateAll(rows=>Promise.all(rows.flatMap(row=>row.getAnimations().map(animation=>animation.finished))));
    assert.equal(await picker.locator('.vp-server').count(),2);
    assert.equal(await picker.locator('input').count(),0);
    assert.equal(await picker.locator('h1').textContent(),'Your servers.');
    assert.equal(await picker.locator('.vp-server').first().evaluate(el=>getComputedStyle(el).borderTopColor),'rgb(39, 52, 72)');
    if(width===1910){assert.ok(Math.abs((await picker.boundingBox()).width-1232)<1);assert.ok(Math.abs((await page.locator('.nav__inner').boundingBox()).width-1280)<1);}
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
    await page.screenshot({path:`output/picker-music-${width}.png`,fullPage:true});
    await picker.locator('.vp-server').first().click();await picker.waitFor({state:'hidden'});
    assert.deepEqual(errors,[]);await page.close();console.log(`Music ${width}px: shared scale, rows, safe names and real selection passed`);
  }
}finally{await browser.close();}

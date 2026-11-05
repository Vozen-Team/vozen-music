import assert from 'node:assert/strict';
import {mkdir,readFile} from 'node:fs/promises';
const {chromium}=await import(process.env.VOZEN_PLAYWRIGHT_MODULE||'playwright');
const browser=await chromium.launch();
const base=process.env.VOZEN_PREVIEW_URL||'http://localhost:4783';
try{
  await mkdir('output',{recursive:true});
  for(const width of [320,768,1440,1910]){
    const page=await browser.newPage({viewport:{width,height:1000}});
    const errors=[];page.on('pageerror',error=>errors.push(error.message));
    await page.route('**/music/api/guilds',route=>route.fulfill({json:{guilds:[{id:'1',name:'Community <safe>'},{id:'2',name:'W'.repeat(100)}]}}));
    if(process.env.SITE_PUBLIC_ROOT)await page.route('**/assets/fonts/*.woff2',async route=>route.fulfill({contentType:'font/woff2',body:await readFile(process.env.SITE_PUBLIC_ROOT+'/assets/fonts/'+new URL(route.request().url()).pathname.split('/').pop())}));
    await page.goto(base+'/music/dashboard');
    const picker=page.locator('#server-picker');await picker.waitFor({state:'visible'});
    assert.equal(await page.locator('#sidebar .install').count(),0);
    assert.equal(new URL(await page.locator('#picker-invite').getAttribute('href')).hostname,'discord.com');
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
    const select=page.locator('#server-select');
    await select.selectOption('2');
    const label=select.locator('selectedcontent .server-option-name');
    await label.waitFor();
    assert.equal(await label.textContent(),'W'.repeat(100));
    assert.equal(await select.getAttribute('title'),'W'.repeat(100));
    const selectedButton=select.locator(':scope > button');
    assert.equal(await selectedButton.evaluate(el=>getComputedStyle(el).borderTopWidth),'0px');
    assert.equal(await selectedButton.evaluate(el=>getComputedStyle(el).backgroundColor),'rgba(0, 0, 0, 0)');
    assert.equal(await selectedButton.evaluate(el=>getComputedStyle(el).paddingInlineStart),'0px');
    assert.equal(await label.evaluate(el=>getComputedStyle(el).textOverflow),'ellipsis');
    assert.ok(await label.evaluate(el=>el.scrollWidth>el.clientWidth));
    const labelBox=await label.boundingBox(), selectBox=await select.boundingBox();
    assert.ok(labelBox.x>=selectBox.x && labelBox.x+labelBox.width<selectBox.x+selectBox.width);
    await select.selectOption('1');
    assert.equal(await select.getAttribute('title'),'Community <safe>');
    assert.deepEqual(errors,[]);await page.close();console.log(`Music ${width}px: shared scale, rows, safe names and real selection passed`);
  }
}finally{await browser.close();}

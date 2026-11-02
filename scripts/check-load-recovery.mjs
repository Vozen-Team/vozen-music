import assert from 'node:assert/strict';
const {chromium}=await import(process.env.VOZEN_PLAYWRIGHT_MODULE||'playwright');
const browser=await chromium.launch();
const base=process.env.VOZEN_PREVIEW_URL||'http://localhost:4783';
try{
 for(const [endpoint,status] of [['guilds',502],['me',503],['guilds',504],['stations',502],['guilds/1/server-settings',503],['guilds','network']]){
  const page=await browser.newPage();let calls=0;
  await page.route(`**/music/api/${endpoint}`,route=>{
   calls++;if(calls===1)return status==='network'?route.abort():route.fulfill({status,contentType:'text/html',body:'<html>Temporary gateway error</html>'});return route.continue();
  });
  const server=['stations','guilds/1/server-settings'].includes(endpoint);
  await page.goto(base+'/music/dashboard'+(server?'?server=1':''));
  const target=page.locator(server?'#editor':'#server-picker');
  await target.waitFor({state:'visible',timeoutMs:7000}).catch(()=>{});
  assert.equal(await target.isVisible(),true,`${endpoint}: transient gateway response must not strand the signed-in user on a blank error page`);
  assert.equal(calls,2);assert.equal(await page.locator('#welcome').isVisible(),false);await page.close();
 }
 for(const [endpoint,status,attempts] of [['me',502,3],['guilds',502,3],['guilds/1/server-settings',502,3],['guilds',200,1]]){
  const page=await browser.newPage();let calls=0,fail=true;
  await page.route(`**/music/api/${endpoint}`,route=>{calls++;return fail?route.fulfill({status,contentType:'text/html',body:'Gateway unavailable'}):route.continue();});
  await page.goto(base+'/music/dashboard'+(endpoint.includes('/1/')?'?server=1':''));
  await page.getByRole('button',{name:'Try again',exact:true}).waitFor({state:'visible'});
  assert.equal(calls,attempts,'Read retries must be bounded');
  assert.equal(await page.locator('#welcome').isVisible(),false,'A gateway error must not log the user out');
  fail=false;await page.getByRole('button',{name:'Try again',exact:true}).click();
  await page.locator(endpoint.includes('/1/')?'#editor':'#server-picker').waitFor({state:'visible'});
  assert.equal(calls,attempts+1);assert.equal(await page.locator('#notice').isVisible(),false);await page.close();
 }
 {
  const page=await browser.newPage();let writes=0;
  await page.route('**/music/api/guilds/1/server-settings',route=>{
   if(route.request().method()!=='PUT')return route.continue();
   writes++;return route.fulfill({status:502,contentType:'text/html',body:'Lost upstream response'});
  });
  await page.goto(base+'/music/dashboard?server=1&page=playback');
  await page.getByLabel('Default and current volume',{exact:true}).fill('41');await page.locator('#music-save').click();
  await page.getByText('The server could not respond. Try again.',{exact:true}).waitFor();
  assert.equal(writes,1,'Writes must never be replayed automatically');
  assert.equal(await page.getByLabel('Default and current volume',{exact:true}).inputValue(),'41');
  assert.equal(await page.getByText('Settings saved for this server.',{exact:true}).isVisible(),false);await page.close();
 }
 {
  const page=await browser.newPage();let calls=0;
  await page.route('**/music/api/me',route=>{calls++;return route.fulfill({status:401,contentType:'text/html',body:'Session expired'});});
  await page.route('https://vozen.org/account/**',route=>route.fulfill({contentType:'text/html',body:'Account sign in'}));
  await page.goto(base+'/music/dashboard');await page.waitForURL('https://vozen.org/account/?music=1');
  assert.equal(calls,1,'Expired sessions must not be retried');await page.close();
 }
 console.log('Music load recovery: transient HTTP/network failures, bounded retries, manual recovery, no false logout, writes not replayed and expired sessions passed.');
}finally{await browser.close();}

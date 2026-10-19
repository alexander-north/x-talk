// Loopback-only fixtures, available exclusively to debug acceptance builds.
import http from 'node:http';
import {readFileSync,existsSync} from 'node:fs';
import path from 'node:path';
const modeFile=process.argv[2];const directory=path.resolve('.tmp/release-assets');
const version=process.env.XTALK_EXPECTED_VERSION||JSON.parse(readFileSync('package.json','utf8')).version;
http.createServer((request,response)=>{
 const mode=existsSync(modeFile)?readFileSync(modeFile,'utf8').trim():'unavailable';
 if(mode==='unavailable'){response.writeHead(503).end();return;}
 if(request.url==='/installer.exe'){
  const file=path.join(directory,`x-talk_${version}_x64-setup.exe`);
  const bytes=existsSync(file)?readFileSync(file):Buffer.from('MZinvalid fixture');
  if(mode==='tampered')bytes[bytes.length-1]^=1;
  response.writeHead(200,{'Content-Length':bytes.length});response.end(bytes);return;
 }
 if(mode==='malformed'){response.end('{broken');return;}
 const signature=path.join(directory,`x-talk_${version}_x64-setup.exe.sig`);
 response.setHeader('Content-Type','application/json');response.end(JSON.stringify({
  version:mode==='current'?'0.0.0':mode==='wrong-version'?'99.0.0':version,notes:'Update fixture: preserve your work.',
  platforms:{'windows-x86_64':{url:'http://127.0.0.1:18979/installer.exe',
   signature:existsSync(signature)?readFileSync(signature,'utf8').trim():'invalid fixture signature'}}
 }));
}).listen(18979,'127.0.0.1');

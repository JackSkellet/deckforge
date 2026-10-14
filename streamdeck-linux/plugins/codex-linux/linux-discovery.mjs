// Prefer the desktop app's matching CLI, then a verified executable on PATH.
import {stat,realpath} from 'node:fs/promises';
import {join,dirname,isAbsolute} from 'node:path';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
const execute=promisify(execFile);
let cached;
export async function discover(environment=process.env) {
  if(cached)return cached;
  const paths=['/usr/lib/chatgpt/resources/codex','/opt/ChatGPT/resources/codex','/opt/Codex/resources/codex'];
  for(const folder of (environment.PATH||'').split(':'))if(isAbsolute(folder))paths.push(join(folder,'codex'));
  for(const candidate of paths) {
    try {
      const file=await realpath(candidate),info=await stat(file),uid=process.getuid();
      if(!info.isFile() || !(info.mode&0o111) || (info.mode&0o022) || ![0,uid].includes(info.uid))continue;
      let parent=dirname(file),safe=true;
      for(;;){const dir=await stat(parent);if((dir.mode&0o022)||![0,uid].includes(dir.uid)){safe=false;break;}if(parent==='/')break;parent=dirname(parent);}
      if(!safe)continue;
      const {stdout}=await execute(file,['--version'],{timeout:3000,maxBuffer:4096,env:environment});
      if(!/^codex-cli\s+\d+\.\d+/m.test(stdout))continue;
      cached=file;return file;
    }catch{}
  }
  return null;
}

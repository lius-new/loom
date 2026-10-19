import {stageLanguagePacks} from './lib/language-packs.mjs';
if(!process.argv[2])throw Error('Supply destination');
stageLanguagePacks({destination:process.argv[2]});

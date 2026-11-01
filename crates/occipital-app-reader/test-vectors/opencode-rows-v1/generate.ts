// Run with bun from this folder, with the prefrontal repository checked out beside
// occipital: it projects the messages below through the plugin's own adapter.
import { transcriptMessageText, transcriptMessageBlockSummaries, transcriptMessageParts, transcriptMessageToolCalls, transcriptMessageFiles } from "../../../../../prefrontal/packages/opencode/src/features/host-bridge/canonical-adapter.ts"
const long = "x".repeat(2048 + 10)
const messages: any[] = [
 {info:{id:"msg_a",role:"assistant",time:{created:1000,completed:1500},providerID:"anthropic",modelID:"claude-opus-5-5"},parts:[
  {id:"prt_0",type:"step-start"},{id:"prt_1",type:"reasoning",text:"thinking"},{id:"prt_2",type:"text",text:"Hello "},
  {id:"prt_3",type:"tool",tool:"bash",callID:"call_1",state:{status:"completed",input:{command:long},output:"done\n"}},
  {id:"prt_4",type:"tool",tool:"read",callID:"call_2",state:{status:"error",input:{},error:"no such file\nstack"}},
  {id:"prt_5",type:"text",text:"<system-reminder>note</system-reminder>"},{id:"prt_6",type:"step-finish"}]},
 {info:{id:"msg_u",role:"user",time:{created:7}},parts:[{id:"prt_0",type:"text",text:"see"},
  {id:"prt_1",type:"file",mime:"image/png",filename:"/Users/someone/shot.png",url:"data:image/png;base64,AAAA"},
  {id:"prt_2",type:"compaction",auto:true}]}]
const out: any[] = []
for (const m of messages) for (const full of [false,true]) {
  const files = transcriptMessageFiles(m), calls = transcriptMessageToolCalls(m)
  out.push({full, role:m.info.role, text:transcriptMessageText(m), blockSummaries:transcriptMessageBlockSummaries(m), parts:transcriptMessageParts(m, full), ...(calls.length?{toolCalls:calls}:{}), ...(files.length?{files}:{})})
}
console.log(JSON.stringify({source:"prefrontal 9b411a17b packages/opencode/src/features/host-bridge/canonical-adapter.ts (transcriptMessage* helpers), run by generate.ts", messages, rows: out}, null, 1))

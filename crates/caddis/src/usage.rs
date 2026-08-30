//! usage.rs — the CLI's discovery surface, ONE LINE PER COMMAND
//! (operator law: never merge lines to make room — split the file
//! instead; that is the tidy shape). This module exists so main.rs
//! never fights its own help text for the 280-line cap.

pub(crate) const USAGE: &str = "\
usage: caddis <attach|rotate|fold|lineage|page|bee|occupancy|ledger|check|worker|brief|fix|build|soul|akis|sentinel|--help|--version>
       caddis attach --harness omp-peleda|claude|qpi [--skill-src DIR]
       caddis rotate ready --lineage <id> --kind omp|claude|qpi --model <id> [--pane <id>]
       caddis rotate arm --lineage <id>
       caddis rotate verify --lineage <id> [--kind omp|claude|qpi] [--force]
       caddis fold threshold --at <1-99>
       caddis fold tick --lineage <id> --used-pct <0-100> [--used-tokens N]
       caddis fold cap --lineage <id> --tokens N
       caddis lineage packet --lineage <id>
       caddis page capture|ref --session <id> (CARD-0155)
       caddis page tick [--session <id>] (CARD-0190)
       caddis page report [--session <id>] (CARD-0160/0168)
       caddis page mode --session <id> [--set page|observe] (CARD-0188)
       caddis page mark --session <id> [--set N] (CARD-0202)
       caddis bee spawn --harness omp|claude|qpi -- <cmd>
       caddis occupancy [--file PATH] (CARD-0333)
       caddis ledger orient [--project NAME] [--since 90d|24h|UNIX]
       caddis check --lineage <id> [--pace run|stop]
       caddis worker board --lineage <id> [--session <id>] [--watch [--interval-ms N] [--frames N]] (CARD-0243)
       caddis worker scan --lineage <id>
       caddis eddy tick --run <id> (--until N | --for-ms T) [--class ...] (CARD-0233)
       caddis beekeeper --lineage <id> [--once] [--interval-secs N] (CARD-0236)
       caddis brief [--lineage <id>] [--voice] (CARD-0252/0264)
       caddis fix <symptom> (CARD-0252)
       caddis build \"<idea>\" (CARD-0252)
       caddis soul compose [--lineage <id>] (CARD-0255)
       caddis akis --card <id> [--file <path>...] (CARD-0271)
       caddis panic --lineage <id> (CARD-0311)
       caddis prove --lineage <id> -- <cmd...> (CARD-0316)
       caddis sentinel audit [--model ID] [--cwd DIR] [--target FILES] \"task\" (CARD-0331)
       caddis sentinel model [--set <id>] (CARD-0331)
       caddis restart enter|spawn|heartbeat|talk --lineage <id> (CARD-0305..0315)
       caddis doctor --lineage <id> (CARD-0310)
";

//! The caddis-voice executable (P3 slice b).
//!
//! Today it is exactly ONE thing: the KILLABLE PLAY CHILD
//! (`play-view`), spawned per attempt by [`caddis_voice::play::AudioOut`]
//! — the organ playing its own binary keeps the child sovereign (no
//! Python, no PortAudio; the daemon's play_proc.py contract ported
//! verbatim: exit 0/10/20/30/40, one JSON line on stdout).
//!
//! The HTTP surface (opener routes, P4) grows here later; unknown argv
//! fails closed with the usage line and exit 2 (a play child that
//! silently tolerated wrong argv would break the exit-code contract).

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "play-view" {
        std::process::exit(caddis_voice::play::play_view(&args[2], &args[3]));
    }
    eprintln!("usage: caddis-voice play-view <wav> <device>");
    std::process::exit(2);
}

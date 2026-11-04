//! Raw bytes read back from the firmware (GPT entries, EFI_LOAD_OPTION) during --undo.
#![no_main]
use libfuzzer_sys::fuzz_target;
use linuxify::fwboot::*;

fuzz_target!(|input: (u32, u32, Vec<u8>)| {
    let (count, size, buf) = input;
    // Keep the loop bound sane: the real caller reads a table the size of the header says.
    let count = count % 4096;
    for (slot, _) in parse_gpt_entries(&buf, count, size % 1024) {
        assert!(slot >= 1 && slot <= count);
    }
    let _ = load_option_description(&buf);
    let parts: Vec<GptEntry> = buf.chunks_exact(16).take(8).enumerate().map(|(i, c)| (i as u32 + 1, c.try_into().unwrap())).collect();
    if let Some((fixed, _changes)) = fix_load_option(&buf, &parts) {
        assert_eq!(fixed.len(), buf.len(), "rewriting a load option must not change its length");
    }
});

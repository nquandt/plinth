//! Source maps of app code (`docs/PRODUCTION.md` L9): a runtime error names
//! the source file and line.
//!
//! The compiler writes the custom section `plinth-srcmap` into `app.wasm`.
//! For each app function, it gives the source line of each range of code:
//! an entry `(offset, line)` applies from `offset` up to the next entry.
//! An offset is relative to the start of the function body (the byte after
//! the body size, where the locals start), so it does not change when other
//! functions move. The function key is the function index in the index
//! space of the module that holds the section (imports included).
//!
//! The host link step (`split::load_app`, `link::link`) copies the map into
//! the linked core module with the indices and offsets of that module. The
//! desktop runner reads it there; the web host reads it from `app.wasm`.
//! An old host ignores the section (it ignores all custom sections of an
//! app module). A release build (`plinth build --release`) removes the
//! section before Binaryen changes the code (`plinth-cli` `optimize.rs`).
//!
//! # Format (version 1)
//!
//! All numbers are unsigned LEB128 (`u`), except the line steps, which are
//! signed LEB128 (`s`).
//!
//! ```text
//! version   u8 = 1
//! files     u count, then count x (u length, UTF-8 bytes)
//! records   u count, then count x record, in increasing function order
//! record    u function step  (from the previous record's function + 1; the first from 0)
//!           u file           (an index into files; only when there are 2 or more files)
//!           u count, then count x (u offset step, s line step)
//! ```
//!
//! In a record, the first offset step is from 0 and the first line step is
//! from 0; each next step is from the previous entry. Offsets increase.

use anyhow::{Context as _, Result};

/// The name of the custom section.
pub const SECTION: &str = "plinth-srcmap";
/// The format version that this code writes and reads.
pub const VERSION: u8 = 1;

/// The source map of one module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SrcMap {
    /// Source file paths, relative to the project (`app/main.tsx`).
    pub files: Vec<String>,
    /// One record for each mapped function, in increasing function order.
    pub funcs: Vec<FuncMap>,
}

/// The lines of one function.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FuncMap {
    /// The function index in the module that holds the map.
    pub func: u32,
    /// An index into `SrcMap::files`.
    pub file: u32,
    /// `(position, line)` in increasing position order. A position is a
    /// byte offset from the start of the body; the compiler uses
    /// instruction indices until it encodes the module.
    pub entries: Vec<(u32, u32)>,
}

impl SrcMap {
    pub fn is_empty(&self) -> bool {
        self.funcs.iter().all(|f| f.entries.is_empty())
    }

    /// The bytes of the custom section.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![VERSION];
        leb(&mut out, self.files.len() as u64);
        for f in &self.files {
            leb(&mut out, f.len() as u64);
            out.extend_from_slice(f.as_bytes());
        }
        let funcs: Vec<&FuncMap> = self.funcs.iter().filter(|f| !f.entries.is_empty()).collect();
        leb(&mut out, funcs.len() as u64);
        let mut next_func = 0u32;
        for f in funcs {
            leb(&mut out, u64::from(f.func - next_func));
            next_func = f.func + 1;
            if self.files.len() > 1 {
                leb(&mut out, u64::from(f.file));
            }
            leb(&mut out, f.entries.len() as u64);
            let (mut off, mut line) = (0u32, 0i64);
            for &(o, l) in &f.entries {
                leb(&mut out, u64::from(o - off));
                sleb(&mut out, i64::from(l) - line);
                off = o;
                line = i64::from(l);
            }
        }
        out
    }

    /// Reads the bytes of the custom section. Never panics: the bytes come
    /// from an app module, which a host does not trust.
    pub fn decode(bytes: &[u8]) -> Result<SrcMap> {
        let mut r = Reader { bytes, pos: 0 };
        anyhow::ensure!(r.byte()? == VERSION, "unknown source map version");
        let mut map = SrcMap::default();
        for _ in 0..r.count()? {
            let len = r.count()?;
            let s = r.take(len)?;
            map.files.push(std::str::from_utf8(s).context("a file name is not UTF-8")?.to_owned());
        }
        let mut next_func = 0u64;
        for _ in 0..r.count()? {
            let func = next_func.checked_add(r.leb()?).filter(|f| *f <= u64::from(u32::MAX)).context("bad function")?;
            next_func = func + 1;
            let file = if map.files.len() > 1 { r.leb()? } else { 0 };
            anyhow::ensure!(file < map.files.len() as u64, "bad file index");
            let mut entries = Vec::new();
            let (mut off, mut line) = (0u64, 0i64);
            for _ in 0..r.count()? {
                off = off.checked_add(r.leb()?).filter(|o| *o <= u64::from(u32::MAX)).context("bad offset")?;
                line = line.checked_add(r.sleb()?).filter(|l| (0..=i64::from(u32::MAX)).contains(l)).context("bad line")?;
                entries.push((off as u32, line as u32));
            }
            map.funcs.push(FuncMap { func: func as u32, file: file as u32, entries });
        }
        anyhow::ensure!(r.pos == bytes.len(), "extra bytes after the source map");
        Ok(map)
    }

    /// The file and line of `offset` in function `func`: the last entry at
    /// or before the offset.
    pub fn lookup(&self, func: u32, offset: u32) -> Option<(&str, u32)> {
        let i = self.funcs.binary_search_by_key(&func, |f| f.func).ok()?;
        let f = &self.funcs[i];
        let n = f.entries.partition_point(|&(o, _)| o <= offset);
        let (_, line) = *f.entries.get(n.checked_sub(1)?)?;
        Some((self.files.get(f.file as usize)?.as_str(), line))
    }

    /// The same map with other function indices and positions: `func` gives
    /// the new index of a function, `pos(old function, old position)` the
    /// new position. A function or an entry that they cannot map is left
    /// out. Entries that come to the same position keep the last one.
    pub fn map(&self, mut func: impl FnMut(u32) -> Option<u32>, mut pos: impl FnMut(u32, u32) -> Option<u32>) -> SrcMap {
        let mut funcs: Vec<FuncMap> = Vec::new();
        for f in &self.funcs {
            let Some(new) = func(f.func) else { continue };
            let mut entries: Vec<(u32, u32)> = Vec::new();
            for &(p, line) in &f.entries {
                let Some(p) = pos(f.func, p) else { continue };
                match entries.last_mut() {
                    Some(last) if last.0 == p => last.1 = line,
                    Some(last) if last.0 > p => {}
                    _ => entries.push((p, line)),
                }
            }
            entries.dedup_by(|b, a| a.1 == b.1);
            funcs.push(FuncMap { func: new, file: f.file, entries });
        }
        funcs.sort_by_key(|f| f.func);
        funcs.dedup_by_key(|f| f.func);
        SrcMap { files: self.files.clone(), funcs }
    }
}

/// The offset of each instruction of a function body (`body` starts with
/// the locals, after the body size), from the start of the body.
pub fn instr_offsets(body: &[u8]) -> Result<Vec<u32>> {
    let body = wasmparser::FunctionBody::new(wasmparser::BinaryReader::new(body, 0));
    let mut ops = body.get_operators_reader()?;
    let mut out = Vec::new();
    while !ops.eof() {
        let (_, off) = ops.read_with_offset()?;
        out.push(off as u32);
    }
    Ok(out)
}

/// True for an instruction where a trap or an interrupt can stop the code:
/// a call (the callee traps, or the stack overflows), a loop (the time
/// limit), `unreachable` (a failed check), an integer division and a bulk
/// memory operation. Only these need a line in the map.
fn can_stop(op: &wasmparser::Operator) -> bool {
    use wasmparser::Operator as O;
    matches!(
        op,
        O::Call { .. }
            | O::CallIndirect { .. }
            | O::ReturnCall { .. }
            | O::ReturnCallIndirect { .. }
            | O::Loop { .. }
            | O::Unreachable
            | O::I32DivS
            | O::I32DivU
            | O::I32RemS
            | O::I32RemU
            | O::I64DivS
            | O::I64DivU
            | O::I64RemS
            | O::I64RemU
            | O::I32TruncF64S
            | O::I32TruncF64U
            | O::I32TruncF32S
            | O::I32TruncF32U
            | O::I64TruncF64S
            | O::I64TruncF64U
            | O::I64TruncF32S
            | O::I64TruncF32U
            | O::MemoryInit { .. }
            | O::MemoryCopy { .. }
            | O::MemoryFill { .. }
    )
}

/// Keeps only the entries that are needed: an entry whose instructions
/// (up to the next entry) cannot stop the code is left out. Its code then
/// maps to the line before it, which no trap can show. `body` starts with
/// the locals, after the body size; positions are instruction indices.
pub fn needed_entries(entries: &[(u32, u32)], body: &[u8]) -> Result<Vec<(u32, u32)>> {
    let body = wasmparser::FunctionBody::new(wasmparser::BinaryReader::new(body, 0));
    let mut ops = body.get_operators_reader()?;
    let mut stops = Vec::new();
    while !ops.eof() {
        stops.push(can_stop(&ops.read()?));
    }
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (k, &(start, line)) in entries.iter().enumerate() {
        let end = entries.get(k + 1).map_or(stops.len(), |e| e.0 as usize).min(stops.len());
        let start = (start as usize).min(end);
        if stops[start..end].iter().any(|s| *s) && out.last().is_none_or(|l| l.1 != line) {
            out.push((start as u32, line));
        }
    }
    Ok(out)
}

/// The index of the instruction at `offset` (or the last one before it).
pub fn instr_at(offsets: &[u32], offset: u32) -> Option<u32> {
    offsets.partition_point(|&o| o <= offset).checked_sub(1).map(|i| i as u32)
}

/// Finds the `plinth-srcmap` section in a module or a component (in any
/// core module inside it) and reads it. `None` when there is none, or when
/// it is not valid: a bad map never stops an app.
pub fn find(bytes: &[u8]) -> Option<SrcMap> {
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        match payload {
            Ok(wasmparser::Payload::CustomSection(c)) if c.name() == SECTION => return SrcMap::decode(c.data()).ok(),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

/// A copy of the module `module` with no `plinth-srcmap` section (for a
/// release build, and for tests of a module with no map). The bytes of
/// every other section stay the same.
pub fn strip(module: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(module.len() >= 8 && module[..4] == *b"\0asm", "not a Wasm module");
    let mut out = module[..8].to_vec();
    let mut r = Reader { bytes: module, pos: 8 };
    while r.pos < module.len() {
        let start = r.pos;
        let id = r.byte()?;
        let size = r.leb()?;
        let content = r.take(size)?;
        if id == 0 {
            let mut c = Reader { bytes: content, pos: 0 };
            let len = c.leb()?;
            if c.take(len)? == SECTION.as_bytes() {
                continue;
            }
        }
        out.extend_from_slice(&module[start..r.pos]);
    }
    Ok(out)
}

fn leb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn sleb(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0);
        if done {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let b = *self.bytes.get(self.pos).context("the source map ends early")?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: u64) -> Result<&[u8]> {
        let end = self.pos.checked_add(usize::try_from(n)?).filter(|e| *e <= self.bytes.len()).context("the source map ends early")?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn leb(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        anyhow::bail!("a number is too long")
    }

    /// A count: no more than the bytes that are left, so a bad count cannot
    /// make a long loop.
    fn count(&mut self) -> Result<u64> {
        let n = self.leb()?;
        anyhow::ensure!(n <= (self.bytes.len() - self.pos) as u64, "a count is too large");
        Ok(n)
    }

    fn sleb(&mut self) -> Result<i64> {
        let mut v = 0i64;
        let mut shift = 0;
        loop {
            let b = self.byte()?;
            anyhow::ensure!(shift < 64, "a number is too long");
            v |= i64::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                if shift < 64 && b & 0x40 != 0 {
                    v |= -1i64 << shift;
                }
                return Ok(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SrcMap {
        SrcMap {
            files: vec!["app/main.tsx".into(), "app/lib.ts".into()],
            funcs: vec![
                FuncMap { func: 40, file: 0, entries: vec![(3, 12), (9, 13), (20, 12), (300, 140)] },
                FuncMap { func: 41, file: 1, entries: vec![(0, 1)] },
                FuncMap { func: 90, file: 0, entries: vec![(5, 7)] },
            ],
        }
    }

    #[test]
    fn encode_and_decode_give_the_same_map() {
        let m = sample();
        assert_eq!(SrcMap::decode(&m.encode()).unwrap(), m);
    }

    #[test]
    fn lookup_takes_the_last_entry_at_or_before_the_offset() {
        let m = sample();
        assert_eq!(m.lookup(40, 2), None, "before the first entry");
        assert_eq!(m.lookup(40, 3), Some(("app/main.tsx", 12)));
        assert_eq!(m.lookup(40, 10), Some(("app/main.tsx", 13)));
        assert_eq!(m.lookup(40, 299), Some(("app/main.tsx", 12)));
        assert_eq!(m.lookup(40, 5000), Some(("app/main.tsx", 140)));
        assert_eq!(m.lookup(41, 0), Some(("app/lib.ts", 1)));
        assert_eq!(m.lookup(42, 0), None);
    }

    #[test]
    fn decode_refuses_bad_bytes_with_no_panic() {
        let good = sample().encode();
        for cut in 0..good.len() {
            assert!(SrcMap::decode(&good[..cut]).is_err(), "cut at {cut}");
        }
        // Every one-byte change: an error or a map, never a panic.
        for i in 0..good.len() {
            for b in [0u8, 0x7f, 0x80, 0xff] {
                let mut bad = good.clone();
                bad[i] = b;
                let _ = SrcMap::decode(&bad);
            }
        }
        assert!(SrcMap::decode(&[2]).is_err(), "an unknown version");
        assert!(SrcMap::decode(&[1, 0xff, 0xff, 0xff, 0xff, 0x0f]).is_err(), "a count larger than the bytes");
    }

    #[test]
    fn map_moves_functions_and_positions() {
        let m = sample().map(|f| (f != 41).then_some(f + 100), |_, p| (p != 9).then_some(p * 2));
        assert_eq!(m.funcs.len(), 2);
        assert_eq!(m.funcs[0].func, 140);
        // (9, 13) is gone; (3, 12) and (20, 12) then have the same line.
        assert_eq!(m.funcs[0].entries, vec![(6, 12), (600, 140)]);
    }
}

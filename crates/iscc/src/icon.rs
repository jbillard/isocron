//! The icon of a program of Windows (D677): a file `.ico` given by
//! `--icon` made the resources of the program, as a resource compiler makes
//! them, in an object of COFF of its own given to the linker: its images
//! (`RT_ICON`, each its number from 1) and their group (`RT_GROUP_ICON`
//! number 1), which the explorer shows and the window of `win32` loads.

/// An image of a file `.ico`: the 12 bytes of its entry that its group
/// repeats (width, height, colours, planes, bits), and its bytes (a PNG or
/// a bitmap without its file header).
struct Image {
    head: [u8; 12],
    bytes: Vec<u8>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The images of the file `.ico` `b`, or why it is not one.
fn images(b: &[u8]) -> Result<Vec<Image>, String> {
    if b.len() < 6 || u16_at(b, 0) != 0 || u16_at(b, 2) != 1 {
        return Err("not a file of icons (.ico): its header is not that of one".into());
    }
    let n = u16_at(b, 4) as usize;
    if n == 0 {
        return Err("a file of icons (.ico) without images".into());
    }
    if b.len() < 6 + 16 * n {
        return Err("a file of icons (.ico) cut before the end of its entries".into());
    }
    let mut out = Vec::new();
    for k in 0..n {
        let e = 6 + 16 * k;
        let size = u32_at(b, e + 8) as usize;
        let at = u32_at(b, e + 12) as usize;
        if size == 0 || at.checked_add(size).is_none_or(|end| end > b.len()) {
            return Err(format!("the image {} of the file of icons (.ico) passes its end", k + 1));
        }
        let mut head = [0u8; 12];
        head.copy_from_slice(&b[e..e + 12]);
        out.push(Image { head, bytes: b[at..at + size].to_vec() });
    }
    Ok(out)
}

/// A table of a directory of resources: its count of entries numbered.
fn directory(out: &mut Vec<u8>, entries: u16) {
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&entries.to_le_bytes());
}

/// An entry of a directory: its number, the place of what it names from
/// the start of the resources (a directory: its high bit set).
fn entry(out: &mut Vec<u8>, id: u32, at: u32, sub: bool) {
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&(at | if sub { 0x8000_0000 } else { 0 }).to_le_bytes());
}

/// The language of the resources: English (United States), as a resource
/// compiler gives them by default.
const LANGUAGE: u32 = 0x0409;

/// The object of COFF (x86-64) holding the resources of the icon of the
/// file `.ico` `ico`: the directories and the entries of the data in
/// `.rsrc$01`, the data in `.rsrc$02` (the linker puts them in this order
/// in `.rsrc`), each entry's place in the image of the program (an RVA) a
/// relocation against `.rsrc$02`.
pub fn icon_object(ico: &[u8]) -> Result<Vec<u8>, String> {
    let ims = images(ico)?;
    let n = ims.len();
    // the data: each image, then the group (its header and an entry of 14
    // bytes for each image, its number in place of its place), each at 8
    let mut data: Vec<u8> = Vec::new();
    let mut places: Vec<(u32, u32)> = Vec::new();
    for im in &ims {
        places.push((data.len() as u32, im.bytes.len() as u32));
        data.extend_from_slice(&im.bytes);
        while data.len() % 8 != 0 {
            data.push(0);
        }
    }
    let mut group: Vec<u8> = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(n as u16).to_le_bytes());
    for (k, im) in ims.iter().enumerate() {
        group.extend_from_slice(&im.head);
        group.extend_from_slice(&((k + 1) as u16).to_le_bytes());
    }
    places.push((data.len() as u32, group.len() as u32));
    data.extend_from_slice(&group);
    while data.len() % 8 != 0 {
        data.push(0);
    }
    // the directories: the root (the two types), the type `RT_ICON` (its
    // images), the type `RT_GROUP_ICON` (its group), one of the language for
    // each resource; then the entries of the data
    let root = 0u32;
    let icons = root + 16 + 8 * 2;
    let groups = icons + 16 + 8 * n as u32;
    let langs = groups + 16 + 8;
    let lang_size = 16 + 8;
    let entries = langs + lang_size * (n as u32 + 1);
    let mut dir: Vec<u8> = Vec::new();
    directory(&mut dir, 2);
    entry(&mut dir, 3, icons, true);
    entry(&mut dir, 14, groups, true);
    directory(&mut dir, n as u16);
    for k in 0..n {
        entry(&mut dir, (k + 1) as u32, langs + lang_size * k as u32, true);
    }
    directory(&mut dir, 1);
    entry(&mut dir, 1, langs + lang_size * n as u32, true);
    for k in 0..=n {
        directory(&mut dir, 1);
        entry(&mut dir, LANGUAGE, entries + 16 * k as u32, false);
    }
    // the entries of the data: the place (relocated: an RVA once linked),
    // the size, the code page, a reserved word
    let mut relocs: Vec<u32> = Vec::new();
    for (at, size) in &places {
        relocs.push(dir.len() as u32);
        dir.extend_from_slice(&at.to_le_bytes());
        dir.extend_from_slice(&size.to_le_bytes());
        dir.extend_from_slice(&[0; 8]);
    }
    // the object: its header, two sections, their bytes, the relocations of
    // the first, two symbols (the sections), an empty table of strings
    let headers = 20 + 40 * 2;
    let dir_at = headers;
    let data_at = dir_at + dir.len();
    let relocs_at = data_at + data.len();
    let symbols_at = relocs_at + 10 * relocs.len();
    let mut o: Vec<u8> = Vec::new();
    o.extend_from_slice(&0x8664u16.to_le_bytes());
    o.extend_from_slice(&2u16.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(symbols_at as u32).to_le_bytes());
    o.extend_from_slice(&2u32.to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    // (initialized data, readable, aligned at 4 then at 8)
    let section = |o: &mut Vec<u8>, name: &[u8; 8], size: usize, at: usize, rel_at: usize, rels: usize, flags: u32| {
        o.extend_from_slice(name);
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&(size as u32).to_le_bytes());
        o.extend_from_slice(&(at as u32).to_le_bytes());
        o.extend_from_slice(&(if rels > 0 { rel_at as u32 } else { 0 }).to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&(rels as u16).to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        o.extend_from_slice(&flags.to_le_bytes());
    };
    section(&mut o, b".rsrc$01", dir.len(), dir_at, relocs_at, relocs.len(), 0x4030_0040);
    section(&mut o, b".rsrc$02", data.len(), data_at, 0, 0, 0x4040_0040);
    o.extend_from_slice(&dir);
    o.extend_from_slice(&data);
    // (IMAGE_REL_AMD64_ADDR32NB against the symbol 1, `.rsrc$02`: the place
    // in the data already in the field, the start of `.rsrc$02` added)
    for r in &relocs {
        o.extend_from_slice(&r.to_le_bytes());
        o.extend_from_slice(&1u32.to_le_bytes());
        o.extend_from_slice(&3u16.to_le_bytes());
    }
    for (k, name) in [b".rsrc$01", b".rsrc$02"].iter().enumerate() {
        o.extend_from_slice(*name);
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&((k + 1) as i16).to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        // (IMAGE_SYM_CLASS_STATIC, no auxiliary record)
        o.push(3);
        o.push(0);
    }
    o.extend_from_slice(&4u32.to_le_bytes());
    Ok(o)
}

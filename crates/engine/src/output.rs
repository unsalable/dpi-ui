//! Islemcinin urettigi paketler icin yeniden kullanilan tampon.
//! Her `process` cagrisinda temizlenir ama kapasitesini korur: isinma sonrasi
//! paket basina heap tahsisi olmaz.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// Bu tamponda kurulan paket. `recalc` ise saglamalar gonderimden once hesaplanir;
    /// `corrupt_tcp_checksum` ise hesaplandiktan sonra bilerek bozulur.
    Built { recalc: bool, corrupt_tcp_checksum: bool },
    /// Yakalanan orijinal paket, degistirilmeden.
    Original,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutItem {
    pub start: usize,
    pub len: usize,
    pub kind: ItemKind,
}

#[derive(Default)]
pub struct Output {
    pub(crate) buf: Vec<u8>,
    items: Vec<OutItem>,
}

impl Output {
    pub fn with_capacity(bytes: usize) -> Self {
        Self { buf: Vec::with_capacity(bytes), items: Vec::with_capacity(16) }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.items.clear();
    }

    pub fn items(&self) -> &[OutItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Ogenin baytlari; `Original` icin bos dilim (cagiran orijinal tamponu kullanir).
    pub fn bytes(&self, item: &OutItem) -> &[u8] {
        &self.buf[item.start..item.start + item.len]
    }

    pub fn bytes_mut(&mut self, item: &OutItem) -> &mut [u8] {
        &mut self.buf[item.start..item.start + item.len]
    }

    /// Ogeyi, orijinal paketi de goz onunde bulundurarak cozer.
    pub fn resolve<'a>(&'a self, item: &OutItem, original: &'a [u8]) -> &'a [u8] {
        match item.kind {
            ItemKind::Original => original,
            ItemKind::Built { .. } => self.bytes(item),
        }
    }

    pub(crate) fn begin(&self) -> usize {
        self.buf.len()
    }

    pub(crate) fn finish(&self, start: usize, recalc: bool, corrupt_tcp_checksum: bool) -> OutItem {
        OutItem { start, len: self.buf.len() - start, kind: ItemKind::Built { recalc, corrupt_tcp_checksum } }
    }

    pub(crate) fn push(&mut self, item: OutItem) {
        self.items.push(item);
    }

    pub(crate) fn push_original(&mut self) {
        self.items.push(OutItem { start: 0, len: 0, kind: ItemKind::Original });
    }
}

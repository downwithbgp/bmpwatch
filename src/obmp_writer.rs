use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

pub const MAGIC: &[u8; 13] = b"BMPDOPENBMP1\n";

pub struct ObmpWriter {
    writer: BufWriter<File>,
    messages_written: u64,
    bytes_written: u64,
}

impl ObmpWriter {
    pub fn create(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(MAGIC)?;
        writer.flush()?;
        Ok(ObmpWriter {
            writer,
            messages_written: 0,
            bytes_written: MAGIC.len() as u64,
        })
    }

    pub fn write_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        let len = payload.len() as u32;
        self.writer.write_all(&len.to_be_bytes())?;
        self.writer.write_all(payload)?;
        self.writer.flush()?;
        self.messages_written += 1;
        self.bytes_written += 4 + payload.len() as u64;
        Ok(())
    }

    pub fn messages_written(&self) -> u64 {
        self.messages_written
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_constant() {
        assert_eq!(MAGIC.len(), 13);
        assert_eq!(MAGIC, b"BMPDOPENBMP1\n");
    }

    #[test]
    fn test_create_writes_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.obmp");
        let w = ObmpWriter::create(&path).unwrap();
        w.finish().unwrap();

        let data = fs::read(&path).unwrap();
        assert_eq!(&data[..13], MAGIC);
        assert_eq!(data.len(), 13);
    }

    #[test]
    fn test_write_single_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.obmp");
        let mut w = ObmpWriter::create(&path).unwrap();

        let payload = b"hello world";
        w.write_frame(payload).unwrap();
        w.finish().unwrap();

        let data = fs::read(&path).unwrap();
        assert_eq!(&data[..13], MAGIC);
        // length = 11 as u32 BE
        let len = u32::from_be_bytes([data[13], data[14], data[15], data[16]]);
        assert_eq!(len, 11);
        assert_eq!(&data[17..], payload);
    }

    #[test]
    fn test_write_multiple_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.obmp");
        let mut w = ObmpWriter::create(&path).unwrap();

        let p1 = b"first";
        let p2 = b"second";
        w.write_frame(p1).unwrap();
        w.write_frame(p2).unwrap();
        w.finish().unwrap();

        let data = fs::read(&path).unwrap();
        assert_eq!(&data[..13], MAGIC);

        // frame 1
        let l1 = u32::from_be_bytes([data[13], data[14], data[15], data[16]]);
        assert_eq!(l1 as usize, p1.len());
        assert_eq!(&data[17..17 + p1.len()], p1);

        // frame 2
        let off = 17 + p1.len();
        let l2 = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        assert_eq!(l2 as usize, p2.len());
        assert_eq!(&data[off + 4..off + 4 + p2.len()], p2);
    }

    #[test]
    fn test_counts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.obmp");
        let mut w = ObmpWriter::create(&path).unwrap();

        w.write_frame(b"a").unwrap();
        w.write_frame(b"bb").unwrap();
        w.write_frame(b"ccc").unwrap();

        assert_eq!(w.messages_written(), 3);
        // 13 (magic) + 3*(4 + 1) + 3*(4 + 2) + 3*(4 + 3)
        // Actually: 13 + (4+1) + (4+2) + (4+3) = 13 + 5 + 6 + 7 = 31
        let expected = 13 + (4 + 1) + (4 + 2) + (4 + 3);
        assert_eq!(w.bytes_written(), expected as u64);

        w.finish().unwrap();
    }

    #[test]
    fn test_creates_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("nested").join("test.obmp");
        let w = ObmpWriter::create(&path).unwrap();
        w.finish().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_empty_payload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.obmp");
        let mut w = ObmpWriter::create(&path).unwrap();

        w.write_frame(b"").unwrap();
        w.finish().unwrap();

        let data = fs::read(&path).unwrap();
        let len = u32::from_be_bytes([data[13], data[14], data[15], data[16]]);
        assert_eq!(len, 0);
        assert_eq!(data.len(), 17); // 13 magic + 4 length + 0 payload
    }
}

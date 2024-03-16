use core::any::Any;

// easy-fs 可以访问实现了 BlockDevice Trait 的块设备驱动程序
pub trait BlockDevice: Send + Sync + Any {
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    fn write_block(&self, block_id: usize, buf: &[u8]);
}

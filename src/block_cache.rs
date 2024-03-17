use alloc::sync::Arc;
use spin::Mutex;

use crate::{block_dev::BlockDevice, BLOCK_SZ};

pub struct BlockCache {
    cache: [u8; BLOCK_SZ],
    block_id: usize,
    block_device: Arc<dyn BlockDevice>,
    modified: bool,
}

// 创建一个 BlockCache 的时候，
// 这将触发一次 read_block 将一个块上的数据从磁盘读到缓冲区cache
impl BlockCache {
    pub fn new(block_id: usize, block_device: Arc<dyn BlockDevice>) -> Self {
        let mut cache = [0u8; BLOCK_SZ];
        block_device.read_block(block_id, &mut cache);
        Self {
            cache,
            block_id,
            block_device,
            modified: false,
        }
    }
}

// 一旦磁盘块已经存在于内存缓存中，CPU 就可以直接访问磁盘块数据了
impl BlockCache {
    fn addr_of_offset(&self, offset: usize) -> usize {
        // addr_of_offset 可以得到一个
        // BlockCache 内部的缓冲区中指定偏移量 offset 的字节地址
        &self.cache[offset] as *const _ as usize
    }
    // get_ref 是一个泛型方法，它可以获取缓冲区中的位于
    // 偏移量 offset 的一个类型为 T 的磁盘上数
    // 据结构的不可变引用
    pub fn get_ref<T>(&self, offset: usize) -> &T
    where
        T: Sized,
    {
        let type_size = core::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SZ);
        let addr = self.addr_of_offset(offset);
        unsafe { &*(addr as *const T) }
    }
    // get_ref 是一个泛型方法，它可以获取缓冲区中的位于
    // 偏移量 offset 的一个类型为 T 的磁盘上数
    // 据结构的可变引用
    pub fn get_mut<T>(&mut self, offset: usize) -> &mut T
    where
        T: Sized,
    {
        let type_size = core::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SZ);
        self.modified = true;
        let addr = self.addr_of_offset(offset);
        unsafe { &mut *(addr as *mut T) }
    }
}

impl BlockCache {
    fn sync(&mut self) {
        if self.modified {
            self.modified = false;
            self.block_device.write_block(self.block_id, &self.cache);
        }
    }
}

impl Drop for BlockCache {
    fn drop(&mut self) {
        self.sync();
    }
}

impl BlockCache {
    pub fn read<T, V>(&self, offset: usize, f: impl FnOnce(&T) -> V) -> V {
        f(self.get_ref(offset))
    }
    pub fn modify<T, V>(&mut self, offset: usize, f: impl FnOnce(&mut T) -> V) -> V {
        f(self.get_mut(offset))
    }
}

// 希望内存中同时只能驻留有限个磁盘块的缓冲区
const BLOCK_CACHE_SIZE: usize = 16;
use alloc::collections::VecDeque;

pub struct BlockCacheManager {
    // 共享引用意义在于块缓存既需要在管理器 BlockCacheManager
    // 保留一个引用，还需要以引用的形式返回给块缓存的请求者
    // 让它可以对块缓存进行访问
    // 互斥访问在单核上的意义在于提供内部可变性通过编译，
    // 在多核环境下则可以帮助我们避免可能的并发冲突
    queue: VecDeque<(usize, Arc<Mutex<BlockCache>>)>,
}

impl BlockCacheManager {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }
}

impl BlockCacheManager {
    pub fn get_block_cache(
        &mut self,
        block_id: usize,
        block_device: Arc<dyn BlockDevice>,
    ) -> Arc<Mutex<BlockCache>> {
        if let Some(pair) = self.queue.iter().find(|pair| pair.0 == block_id) {
            Arc::clone(&pair.1)
        } else {
            if self.queue.len() == BLOCK_CACHE_SIZE {
                /*
                此时队头对应的块缓存可能仍在使用：判断的标志是其强引用计数 ≥ 2 ，即
                除了块缓存管理器保留的一份副本之外，在外面还有若干份副本正在使用。
                因此，我们的做法是从队头遍历到队尾找到第一个强引用计数恰好为 1
                的块缓存并将其替换出去。
                                */
                if let Some((idx, _)) = self
                    .queue
                    .iter()
                    .enumerate()
                    .find(|(_, pair)| Arc::strong_count(&pair.1) == 1)
                {
                    self.queue.drain(idx..=idx);
                } else {
                    panic!("Run Out of BlockCache!");
                }
            }
            let block_cache = Arc::new(Mutex::new(BlockCache::new(
                block_id, Arc::clone(&block_device))));
            self.queue.push_back((block_id,Arc::clone(&block_cache)));
            block_cache
        }
    }
}

use lazy_static::*;
lazy_static! {
    pub static ref BLOCK_CACHE_MANAGER: Mutex<BlockCacheManager> =
        Mutex::new(BlockCacheManager::new());
}
// 公布到外部的API
pub fn get_block_cache(
    block_id: usize,
    block_device: Arc<dyn BlockDevice>,
) -> Arc<Mutex<BlockCache>> {
    BLOCK_CACHE_MANAGER
        .lock()
        .get_block_cache(block_id, block_device)
}

/// Sync all block cache to block device
pub fn block_cache_sync_all() {
    let manager = BLOCK_CACHE_MANAGER.lock();
    for (_, cache) in manager.queue.iter() {
        cache.lock().sync();
    }
}

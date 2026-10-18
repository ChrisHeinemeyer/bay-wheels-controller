use alloc::boxed::Box;
use core::{
    cell::{RefCell, UnsafeCell},
    ffi::c_void,
    ptr::NonNull,
};

use esp_hal::time::{Duration, Instant};
use esp_radio_rtos_driver::{
    register_timer_implementation,
    timer::{TimerImplementation, TimerPtr},
};
use esp_sync::NonReentrantMutex;

use crate::{
    SCHEDULER,
    task::{TaskExt, TaskPtr},
};

static TIMER_QUEUE: TimerQueue = TimerQueue::new();

struct TimerQueueInner {
    // A linked list of active timers
    head: Option<NonNull<Timer>>,
    next_wakeup: u64,
    task: Option<TaskPtr>,
}

unsafe impl Send for TimerQueueInner {}

impl TimerQueueInner {
    const fn new() -> Self {
        Self {
            head: None,
            next_wakeup: 0,
            task: None,
        }
    }

    fn enqueue(&mut self, timer: &Timer) {
        let head = self.head;
        let props = timer.properties(self);
        let due = props.next_due;

        if !props.enqueued {
            props.enqueued = true;

            props.next = head;
            self.head = Some(NonNull::from(timer));
        }

        if let Some(task) = self.task {
            if due < self.next_wakeup {
                self.next_wakeup = due;
                task.resume();
            }
        } else {
            // create the timer task
            let task_ptr =
                SCHEDULER.create_task("timer", timer_task, core::ptr::null_mut(), 8192, 2, None);
            self.task = Some(task_ptr);
            self.next_wakeup = due;
        }
    }

    fn dequeue(&mut self, timer: &Timer) -> bool {
        let mut current = self.head;
        let mut prev: Option<NonNull<Timer>> = None;

        // Scan through the queue until we find the timer
        while let Some(current_timer) = current {
            if core::ptr::eq(current_timer.as_ptr(), timer) {
                // If we find the timer, remove it from the queue by bypassing it in the linked
                // list. The previous element, if any, will point at the next element.

                let timer_props = timer.properties(self);
                let next = timer_props.next.take();
                timer_props.enqueued = false;

                if let Some(mut p) = prev {
                    unsafe { p.as_mut().properties(self).next = next };
                } else {
                    self.head = next;
                }
                return true;
            }

            prev = current;
            current = unsafe { current_timer.as_ref().properties(self).next };
        }

        false
    }
}

struct TimerQueue {
    inner: NonReentrantMutex<TimerQueueInner>,
}

unsafe impl Send for TimerQueue {}

impl TimerQueue {
    const fn new() -> Self {
        Self {
            inner: NonReentrantMutex::new(TimerQueueInner::new()),
        }
    }

    /// Trigger due timers.
    ///
    /// The timer queue needs to be re-processed when a new timer is armed, because the new timer
    /// may need to be triggered before the next scheduled wakeup.
    fn process(&self) {
        debug!("Processing timers");
        let mut timers = self.inner.with(|q| {
            q.next_wakeup = u64::MAX;
            q.head.take()
        });
        let mut fired_any = false;

        while let Some(current) = timers {
            debug!("Checking timer: {:x}", current.addr());
            let current_timer = unsafe { current.as_ref() };

            let run_callback = self.inner.with(|q| {
                let props = current_timer.properties(q);

                // Remove current timer from the list.
                timers = props.next.take();

                if !props.is_active || props.drop {
                    debug!(
                        "Timer {:x} is inactive or dropped",
                        current_timer as *const _ as usize
                    );
                    return false;
                }

                if props.next_due > crate::now() {
                    // Not our time yet.
                    debug!(
                        "Timer {:x} is not due yet",
                        current_timer as *const _ as usize
                    );
                    return false;
                }

                if props.periodic {
                    props.next_due += props.period;
                }
                props.is_active = props.periodic;
                true
            });

            if run_callback {
                fired_any = true;
                stats::record_fire(current_timer.func);
                debug!("Triggering timer: {:x}", current_timer as *const _ as usize);
                (current_timer.callback.borrow_mut())();
            }

            self.inner.with(|q| {
                let props = current_timer.properties(q);
                // Set this AFTER the callback so that the callback doesn't leave us in an unknown
                // "queued?" state.
                props.enqueued = false;

                if props.drop {
                    debug!("Dropping timer {:x} (delayed)", current.addr());
                    let boxed = unsafe { Box::from_raw(current.as_ptr()) };
                    core::mem::drop(boxed);
                } else if props.is_active {
                    let next_due = props.next_due;
                    if next_due < q.next_wakeup {
                        q.next_wakeup = next_due;
                    }

                    debug!("Re-queueing timer {:x}", current_timer as *const _ as usize);
                    q.enqueue(current_timer);
                } else {
                    debug!("Timer {:x} inactive", current_timer as *const _ as usize);
                }
            });
        }

        stats::record_pass(fired_any);

        self.inner.with(|q| {
            let next_wakeup = q.next_wakeup;
            debug!("next_wakeup: {}", next_wakeup);
            SCHEDULER.sleep_until(Instant::EPOCH + Duration::from_micros(next_wakeup));
        });
    }
}

struct TimerProperties {
    is_active: bool,
    next_due: u64,
    period: u64,
    periodic: bool,
    drop: bool,

    enqueued: bool,
    next: Option<NonNull<Timer>>,
}

struct TimerQueueCell<T>(UnsafeCell<T>);

impl<T> TimerQueueCell<T> {
    const fn new(inner: T) -> Self {
        Self(UnsafeCell::new(inner))
    }

    fn get_mut<'a>(&'a self, _q: &'a mut TimerQueueInner) -> &'a mut T {
        unsafe { &mut *self.0.get() }
    }
}

pub struct Timer {
    callback: RefCell<Box<dyn FnMut() + Send>>,
    /// PATCHED (bay-wheels-controller): the C callback's address, for `stats`.
    func: usize,
    // Timer properties, not available in `callback` due to how the timer is constructed.
    timer_properties: TimerQueueCell<TimerProperties>,
}

impl Timer {
    pub fn new(callback: Box<dyn FnMut() + Send>) -> Self {
        Timer {
            callback: RefCell::new(callback),
            func: 0,
            timer_properties: TimerQueueCell::new(TimerProperties {
                is_active: false,
                next_due: 0,
                period: 0,
                periodic: false,
                drop: false,

                enqueued: false,
                next: None,
            }),
        }
    }

    unsafe fn from_ptr<'a>(ptr: TimerPtr) -> &'a Self {
        unsafe { ptr.cast::<Self>().as_mut() }
    }

    fn arm(&self, q: &mut TimerQueueInner, timeout: u64, periodic: bool) {
        let next_due = crate::now() + timeout;

        let props = self.properties(q);
        props.is_active = true;
        props.next_due = next_due;
        props.period = timeout;
        props.periodic = periodic;

        q.enqueue(self);
    }

    fn is_active(&self, q: &mut TimerQueueInner) -> bool {
        self.properties(q).is_active
    }

    fn disarm(&self, q: &mut TimerQueueInner) {
        self.properties(q).is_active = false;

        // We don't dequeue the timer - processing the queue will just skip it. If we re-arm,
        // the timer may already be in the queue.
    }

    fn properties<'a>(&'a self, q: &'a mut TimerQueueInner) -> &'a mut TimerProperties {
        self.timer_properties.get_mut(q)
    }
}

impl TimerImplementation for Timer {
    fn create(func: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> TimerPtr {
        // TODO: get rid of the inner box (or its heap allocation) somehow
        struct CCallback {
            func: unsafe extern "C" fn(*mut c_void),
            data: *mut c_void,
        }
        unsafe impl Send for CCallback {}

        impl CCallback {
            unsafe fn call(&mut self) {
                unsafe { (self.func)(self.data) }
            }
        }

        let mut callback = CCallback { func, data };

        let func_addr = func as usize;
        let mut timer = Box::new(Timer::new(Box::new(move || unsafe { callback.call() })));
        timer.func = func_addr;
        let ptr = NonNull::from(Box::leak(timer)).cast();
        debug!("Created timer: {:x}", ptr.addr());
        ptr
    }

    unsafe fn delete(timer: TimerPtr) {
        debug!("Deleting timer: {:x}", timer.addr());
        TIMER_QUEUE.inner.with(|q| {
            let timer = unsafe { Box::from_raw(timer.cast::<Timer>().as_ptr()) };

            // There are two cases:
            // - We can remove the timer from the queue - we can drop it.
            // - We can't remove the timer from the queue. There are the following cases:
            //   - The timer isn't in the queue. We can drop it.
            //   - The timer is in the queue and the queue is being processed. We need to mark it to
            //     be dropped by the timer queue.
            // PATCHED (bay-wheels-controller): upstream 0.2.0 marked *every*
            // timer that wasn't in the queue as `drop` and forgot it, but the
            // queue only ever drops timers it is processing — one that wasn't
            // queued at all (fired one-shot, never armed) leaked ~48 bytes.
            // The WiFi blob creates/deletes several per connect cycle, so the
            // heap ran out after ~2.5h. Fixed upstream in esp-rtos 0.3.0
            // ("Fixed a memory leak of 48 bytes when deleting esp-radio
            // timers", #4541). `enqueued` stays true while the queue is
            // processing a detached timer, so it tells the two cases apart.
            if q.dequeue(&timer) || !timer.properties(q).enqueued {
                core::mem::drop(timer);
            } else {
                timer.properties(q).drop = true;
                core::mem::forget(timer);
            }
        })
    }

    unsafe fn arm(timer: TimerPtr, timeout: u64, periodic: bool) {
        debug!(
            "Arming {:?} for {} us, periodic = {:?}",
            timer, timeout, periodic
        );
        let timer = unsafe { Timer::from_ptr(timer) };
        TIMER_QUEUE.inner.with(|q| timer.arm(q, timeout, periodic))
    }

    unsafe fn is_active(timer: TimerPtr) -> bool {
        debug!("Checking if timer {:?} is active", timer);
        let timer = unsafe { Timer::from_ptr(timer) };
        TIMER_QUEUE.inner.with(|q| timer.is_active(q))
    }

    unsafe fn disarm(timer: TimerPtr) {
        debug!("Disarming {:?}", timer);
        let timer = unsafe { Timer::from_ptr(timer) };
        TIMER_QUEUE.inner.with(|q| timer.disarm(q))
    }
}

register_timer_implementation!(Timer);

/// Entry point for the timer task responsible for handling scheduled timer
/// events.
///
/// The timer task is created when the first timer is armed.
pub(crate) extern "C" fn timer_task(_: *mut c_void) {
    loop {
        TIMER_QUEUE.process();
    }
}

/// PATCHED (bay-wheels-controller): counts timer callbacks by C function
/// address while `ENABLED`, to find which radio timers keep waking the CPU
/// once the radio is stopped. Map the addresses to symbols with the ELF.
pub mod stats {
    use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

    /// Distinct callback functions tracked.
    pub const SLOTS: usize = 16;

    /// Count only while set (the firmware sets it while light-sleeping).
    pub static ENABLED: AtomicBool = AtomicBool::new(false);
    static FUNCS: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
    static FIRES: [AtomicU32; SLOTS] = [const { AtomicU32::new(0) }; SLOTS];
    /// Fires whose function didn't fit in the table.
    static OTHER: AtomicU32 = AtomicU32::new(0);
    /// `process()` passes, and those that ran no callback (e.g. woken for a
    /// timer that was disarmed before it was due).
    static PASSES: AtomicU32 = AtomicU32::new(0);
    static EMPTY_PASSES: AtomicU32 = AtomicU32::new(0);

    pub(super) fn record_fire(func: usize) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        for i in 0..SLOTS {
            let cur = FUNCS[i].load(Ordering::Relaxed);
            if cur == func
                || (cur == 0
                    && FUNCS[i]
                        .compare_exchange(0, func, Ordering::Relaxed, Ordering::Relaxed)
                        .map_or_else(|v| v == func, |_| true))
            {
                FIRES[i].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        OTHER.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_pass(fired_any: bool) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        PASSES.fetch_add(1, Ordering::Relaxed);
        if !fired_any {
            EMPTY_PASSES.fetch_add(1, Ordering::Relaxed);
        }
    }

    static SLEEP_ENTRIES: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(usize::MAX) }; SLOTS];
    static SLEEPS: [AtomicU32; SLOTS] = [const { AtomicU32::new(0) }; SLOTS];

    /// Counts a timed sleep by the thread whose entry function is `entry`
    /// (0 = main task, which runs the embassy executor).
    pub(crate) fn record_sleep(entry: usize) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        for i in 0..SLOTS {
            let cur = SLEEP_ENTRIES[i].load(Ordering::Relaxed);
            if cur == entry
                || (cur == usize::MAX
                    && SLEEP_ENTRIES[i]
                        .compare_exchange(usize::MAX, entry, Ordering::Relaxed, Ordering::Relaxed)
                        .map_or_else(|v| v == entry, |_| true))
            {
                SLEEPS[i].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
    }

    static ARMS: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];

    /// Counts an RTOS alarm armed for: 0 a sleeping thread, 1 the time
    /// slice, 2 the embassy timer queue, 3 nothing / unknown.
    pub(crate) fn record_arm(source: usize) {
        if ENABLED.load(Ordering::Relaxed) {
            ARMS[source].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Alarms armed by source (see `record_arm`).
    pub fn arms() -> [u32; 4] {
        core::array::from_fn(|i| ARMS[i].load(Ordering::Relaxed))
    }

    /// `(thread entry address, timed sleeps)` per used slot.
    pub fn sleeps() -> [(usize, u32); SLOTS] {
        let mut out = [(usize::MAX, 0); SLOTS];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = (
                SLEEP_ENTRIES[i].load(Ordering::Relaxed),
                SLEEPS[i].load(Ordering::Relaxed),
            );
        }
        out
    }

    /// `(function address, fires)` per slot (unused slots are `(0, 0)`), plus
    /// (fires of other functions, passes, empty passes). Totals since boot.
    pub fn snapshot() -> ([(usize, u32); SLOTS], (u32, u32, u32)) {
        let mut out = [(0, 0); SLOTS];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = (FUNCS[i].load(Ordering::Relaxed), FIRES[i].load(Ordering::Relaxed));
        }
        (
            out,
            (
                OTHER.load(Ordering::Relaxed),
                PASSES.load(Ordering::Relaxed),
                EMPTY_PASSES.load(Ordering::Relaxed),
            ),
        )
    }
}

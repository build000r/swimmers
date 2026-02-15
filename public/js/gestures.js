// Gesture handling — swipe-right to go back, long-press to create session

window.Gestures = {
  LONG_PRESS_MS: 500,

  // Long press on field to create a new session
  initFieldLongPress(fieldEl, onCreateSession) {
    let timer = null;

    function start(e) {
      const target = e.target || e.srcElement;
      if (target.closest && target.closest('.thronglet')) return;
      timer = setTimeout(() => {
        timer = null;
        if (navigator.vibrate) navigator.vibrate(50);
        onCreateSession();
      }, window.Gestures.LONG_PRESS_MS);
    }

    function cancel() {
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
    }

    fieldEl.addEventListener('contextmenu', (e) => e.preventDefault());
    fieldEl.addEventListener('touchstart', start, { passive: true });
    fieldEl.addEventListener('touchmove', cancel, { passive: true });
    fieldEl.addEventListener('touchend', cancel, { passive: true });
    fieldEl.addEventListener('mousedown', start);
    fieldEl.addEventListener('mousemove', cancel);
    fieldEl.addEventListener('mouseup', cancel);
    fieldEl.addEventListener('mouseleave', cancel);
  },

  // Swipe right from left edge to go back to overview
  initSwipeBack(terminalViewEl, onSwipeBack) {
    let touchStartX = 0;

    terminalViewEl.addEventListener('touchstart', (e) => {
      touchStartX = e.touches[0].clientX;
    }, { passive: true });

    terminalViewEl.addEventListener('touchend', (e) => {
      const dx = e.changedTouches[0].clientX - touchStartX;
      if (touchStartX < 40 && dx > 80) {
        onSwipeBack();
      }
    }, { passive: true });
  },
};

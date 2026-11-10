/* Size the native window to the measured content box.
   Background text that does not change that box does not produce a new size. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.uploaderFit = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  function clamp(n, min, max) {
    return Math.min(max, Math.max(min, n));
  }

  function contentFitSize(content, limits) {
    const minWidth = limits.minWidth;
    const minHeight = limits.minHeight;
    const maxWidth = Math.max(minWidth, limits.maxWidth);
    const maxHeight = Math.max(minHeight, limits.maxHeight);
    const width = clamp(Math.ceil(content.width), minWidth, maxWidth);
    const height = clamp(Math.ceil(content.height), minHeight, maxHeight);
    return {
      width,
      height,
      clipped: content.width > maxWidth + 0.5 || content.height > maxHeight + 0.5,
    };
  }

  function fitChanged(previous, next) {
    if (!previous) return true;
    return previous.width !== next.width || previous.height !== next.height;
  }

  return { contentFitSize, fitChanged };
});

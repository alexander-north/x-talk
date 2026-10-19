import type {Action} from 'svelte/action';

// Measure actual wrapped lines, including explicit newlines and the leading ellipsis.
// The saved text remains untouched: card actions copy through the native entry ID.
export const tailText: Action<HTMLDivElement, string> = (node, initial) => {
 let text = initial;
 let destroyed = false;
 const span = document.createElement('span');
 node.append(span);
 function fit() {
  if (destroyed) return;
  const style = getComputedStyle(node);
  const height = node.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
  if (node.clientWidth === 0 || height <= 0) return;
  span.textContent = text;
  if (span.offsetHeight <= height) return;
  // Grapheme boundaries avoid splitting surrogate pairs, accents, or emoji sequences.
  const starts = Array.from(new Intl.Segmenter(undefined, {granularity:'grapheme'}).segment(text), part => part.index);
  let lo = 0, hi = starts.length;
  const suffix = (count:number) => count ? '… ' + text.slice(starts[starts.length-count]).trimStart() : '…';
  while (lo < hi) {
   const count = Math.ceil((lo + hi) / 2);
   span.textContent = suffix(count);
   if (span.offsetHeight <= height) lo = count; else hi = count - 1;
  }
  span.textContent = suffix(lo);
 }
 const observer = new ResizeObserver(fit);
 observer.observe(node);
 void document.fonts.ready.then(fit);
 document.fonts.addEventListener('loadingdone', fit);
 fit();
 return {
  update(value) { text = value; fit(); },
  destroy() { destroyed = true; observer.disconnect(); document.fonts.removeEventListener('loadingdone', fit); span.remove(); }
 };
};

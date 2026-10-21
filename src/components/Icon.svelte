<script lang="ts">
 let {name, size=20}: {name:string;size?:number}=$props();
 const paths:Record<string,string>={
  mic:'M12 15a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v7a3 3 0 0 0 3 3ZM5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8',
  heart:'M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1.1-1.1a5.5 5.5 0 0 0-7.8 7.8L12 21l8.8-8.6a5.5 5.5 0 0 0 0-7.8Z',
  list:'M9 6h12M9 12h12M9 18h12M3 6h.01M3 12h.01M3 18h.01',
  settings:'M9 3h6l.6 3 2.4 1.4 2.9-.9 3 5.2-2.3 2.1v2.8l2.3 2.1-3 5.2-2.9-.9-2.4 1.4-.6 3H9l-.6-3L6 23l-2.9.9-3-5.2L2.4 16v-2.8L.1 11.1l3-5.2 2.9.9 2.4-1.4Z',
  gear:'m9 3-.6 2.1-2 1.2-2.1-.5-2 3.4L4 10.8v2.4l-1.7 1.6 2 3.4 2.1-.5 2 1.2L9 21h4l.6-2.1 2-1.2 2.1.5 2-3.4-1.7-1.6v-2.4l1.7-1.6-2-3.4-2.1.5-2-1.2L13 3ZM14 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
  copy:'M9 9h12v12H9zM15 5V3H3v12h2',
  warning:'M12 3 2 21h20ZM12 9v5m0 3v.1',
  check:'m5 12 4 4L20 5',
  close:'m6 6 12 12M6 18 18 6',
  play:'m8 5 12 7-12 7Z',pause:'M8 5v14M16 5v14',stop:'M6 6h12v12H6z',
  download:'M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5',
  upload:'M12 16V4m-5 5 5-5 5 5M4 16v5h16v-5',
  trash:'M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7',
  edit:'m15 4 5 5M4 20l5-1L21 7a2.8 2.8 0 0 0-4-4L5 15Z',
  retry:'M20 7v5h-5M20 12a8 8 0 1 0-1 5',
  spark:'m12 2 2.5 7.5L22 12l-7.5 2.5L12 22l-2.5-7.5L2 12l7.5-2.5Z',
  search:'M21 21l-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0',
  columns:'M3 4h18v16H3zM9 4v16M15 4v16',
  compact:'M3 3h18v18H3zM12 3v18M3 12h9',
  expand:'M8 3H3v5M16 3h5v5M21 16v5h-5M3 16v5h5',
  fullscreen:'M8 3H3v5m13-5h5v5M3 16v5h5m8 0h5v-5',
  auto:'M3 4h18v13H3zM8 21h8m-4-4v4M12 4v13M16 8h2m-2 4h2',
  bookmark:'M6 3h12v19l-6-4-6 4Z',
  sun:'M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0',
  moon:'M21 13a9 9 0 0 1-10-10 9 9 0 1 0 10 10Z',
  github:'M9 19c-4.5 1.4-4.5-2-6-2m12 5v-3.8a3.3 3.3 0 0 0-.9-2.6c3-.3 6.2-1.5 6.2-6.8A5.3 5.3 0 0 0 19 5.1 5 5 0 0 0 18.9 1S17.7.6 15 2.5a13.4 13.4 0 0 0-7 0C5.3.6 4.1 1 4.1 1A5 5 0 0 0 4 5.1a5.3 5.3 0 0 0-1.4 3.7c0 5.3 3.2 6.5 6.2 6.8a3.3 3.3 0 0 0-.8 2.6V22',
  arrow:'m6 9 6 6 6-6',left:'m15 6-6 6 6 6',right:'m9 6 6 6-6 6',
 };
</script>
<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={paths[name]??paths.heart}/></svg>

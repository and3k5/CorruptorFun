<script setup>
import { RouterLink, RouterView } from 'vue-router';
</script>

<template>
  <header class="top">
    <div class="wrap bar">
      <RouterLink to="/" class="brand"><span class="g1">Corr</span><span class="g2">u</span><span class="g3">ptor</span>Fun</RouterLink>
      <span class="note">Everything runs in your browser. Your files never leave it.</span>
    </div>
  </header>
  <main class="wrap">
    <RouterView />
  </main>
  <footer class="wrap note foot">
    For entertainment. Bring your own legally dumped files; none are included or downloaded.
    Not affiliated with Sony or Microsoft.
  </footer>
</template>

<style scoped>
.top { border-bottom: 1px solid var(--line); margin-bottom: 24px; }
.bar { display: flex; flex-wrap: wrap; gap: 4px 16px; align-items: baseline; justify-content: space-between; padding-top: 14px; padding-bottom: 14px; }
.brand { font-weight: 800; font-size: 1.25rem; color: var(--text); text-decoration: none; letter-spacing: 0.02em; }
.g1 { color: var(--accent); }
.g2 { color: var(--accent-2); display: inline-block; transform: translateY(2px) skewX(-12deg); }
.g3 { color: var(--accent); }
.foot { margin-top: 40px; padding-bottom: 32px; }
</style>

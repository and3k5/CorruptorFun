<script setup>
import { RouterLink } from 'vue-router';
import { COLLECTION } from '../collection.js';
</script>

<template>
  <section class="intro">
    <h1>Break things, beautifully.</h1>
    <p class="muted">A growing collection of toys that corrupt data and render whatever is left. Pick one.</p>
  </section>
  <ul class="grid">
    <li v-for="item in COLLECTION" :key="item.id">
      <RouterLink :to="item.route" class="card panel">
        <h2>{{ item.title }}</h2>
        <p class="muted">{{ item.blurb }}</p>
        <div class="row"><span v-for="t in item.tags" :key="t" class="tag">{{ t }}</span></div>
      </RouterLink>
    </li>
  </ul>
</template>

<style scoped>
.intro { margin-bottom: 24px; }
.grid { list-style: none; padding: 0; margin: 0; display: grid; gap: 16px; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); }
.card { display: block; color: inherit; text-decoration: none; height: 100%; transition: border-color 0.15s, transform 0.15s; }
.card:hover { border-color: var(--accent); transform: translateY(-2px); }
</style>

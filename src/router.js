import { createRouter, createWebHashHistory } from 'vue-router';
import HomeView from './views/HomeView.vue';

// Hash history: GitHub Pages has no server rewrites, so deep links must not
// depend on the path.
export default createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', component: HomeView },
    { path: '/bios', component: () => import('./views/BiosCorruptor.vue') },
    { path: '/:pathMatch(.*)*', redirect: '/' },
  ],
  scrollBehavior: () => ({ top: 0 }),
});

import { createRouter, createWebHistory } from "vue-router";
import UserPage from "./UserPage.vue";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/users/:userId",
      component: UserPage,
    },
  ],
});

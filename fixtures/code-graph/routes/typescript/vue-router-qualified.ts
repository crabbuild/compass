import { createRouter, createWebHistory } from "vue-router";

function QualifiedUserPage() {
  return null;
}

export const qualifiedRouter = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/qualified-users/:userId",
      component: QualifiedUserPage,
    },
  ],
});

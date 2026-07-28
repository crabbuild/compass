package routes

import "github.com/go-chi/chi/v5"

func showUser(w any, r any) {}

func Routes(r chi.Router) {
	r.Get("/users/{id}", showUser)
}

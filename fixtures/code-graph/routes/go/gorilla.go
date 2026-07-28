package routes

import "github.com/gorilla/mux"

func updateUser(w any, r any) {}

func Routes(r *mux.Router) {
	r.HandleFunc("/users/{id}", updateUser).Methods("PUT", "PATCH")
}

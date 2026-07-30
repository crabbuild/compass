package routes

import "github.com/gin-gonic/gin"

func auth(c *gin.Context) {}
func listUsers(c *gin.Context) {}
func createUser(c *gin.Context) {}

func Routes(r *gin.Engine) {
	api := r.Group("/api")
	api.GET("/users", auth, listUsers)
	api.POST("/users", createUser)
}

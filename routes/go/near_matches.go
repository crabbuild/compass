package routes

type fakeRouter struct{}

func (fakeRouter) GET(path string, handler any) {}
func handler() {}

func Routes(r fakeRouter) {
	r.GET("/not-a-route", handler)
}

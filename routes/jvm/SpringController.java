package example;

import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.RequestMethod;
import org.springframework.web.bind.annotation.RestController;

@RestController
@RequestMapping("/api")
public class SpringController {
    @GetMapping("/users/{id}")
    public String show() { return "ok"; }

    @PostMapping(path = "/users")
    public String create() { return "ok"; }

    @RequestMapping(path = "/search", method = {RequestMethod.GET, RequestMethod.POST})
    public String search() { return "ok"; }
}

package example.kotlin

import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@RequestMapping("/kotlin")
class SpringKotlinController {
    @GetMapping("/users/{id}")
    fun show(id: Long): String = id.toString()
}

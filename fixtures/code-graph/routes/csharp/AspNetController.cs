using Microsoft.AspNetCore.Mvc;

[ApiController]
[Route("api/[controller]")]
public class UsersController : ControllerBase
{
    [HttpGet("{id}")]
    public object Show(string id) => new();

    [HttpPost]
    public object Create() => new();

    [HttpGet("/status")]
    public object Status() => new();

    [HttpGet]
    [Route("health")]
    public object Health() => new();
}

public class QualificationPayload {}

public class QualificationTypes
{
    public QualificationPayload Echo(QualificationPayload value) => value;
}

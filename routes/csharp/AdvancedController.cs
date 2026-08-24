using Microsoft.AspNetCore.Mvc;
using Get = Microsoft.AspNetCore.Mvc.HttpGetAttribute;

[Route("v2/[controller]")]
public class PlainController
{
    [Get("[action]")]
    public object List() => new();

    [AcceptVerbs("GET", "POST", Route = "multi")]
    public object Multi() => new();

    [HttpGet("~/ready")]
    public object Ready() => new();

    [NonAction]
    [HttpGet("hidden")]
    public object Hidden() => new();
}

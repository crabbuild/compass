public class HttpGetAttribute {}

public class FakeController
{
    [HttpGet("/not-a-route")]
    public void Show() {}
}

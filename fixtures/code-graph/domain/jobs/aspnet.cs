using Microsoft.Extensions.Hosting;

public class RebuildIndex : BackgroundService
{
    protected override Task ExecuteAsync(CancellationToken token) => Task.CompletedTask;
}

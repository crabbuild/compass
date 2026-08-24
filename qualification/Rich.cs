namespace Qualification;

public interface IService {
    string Run(int value);
}

public class BaseService {
    public virtual string Run(int value) => value.ToString();
}

public class Service : BaseService, IService {
    public const int Limit = 4;
    private readonly int count;
    public int Count { get; set; }

    public Service(int count) {
        this.count = count;
    }

    public override string Run(int value) => value.ToString();
}

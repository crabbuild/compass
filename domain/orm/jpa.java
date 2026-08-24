import jakarta.persistence.Entity;
import jakarta.persistence.Table;

@Entity
@Table(name = "payments", schema = "billing")
class Payment {}

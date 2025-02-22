# docs: begin-b1
from sqlalchemy import create_engine, not_, or_, and_, false
from sqlalchemy.types import String, Boolean, Integer
from sqlalchemy.schema import Column, ForeignKey
from sqlalchemy.orm import sessionmaker, relationship
from sqlalchemy.ext.declarative import declarative_base

Base = declarative_base()


class Organization(Base):
    __tablename__ = "orgs"

    id = Column(String(), primary_key=True)


# Repositories belong to Organizations
class Repository(Base):
    __tablename__ = "repos"

    id = Column(String(), primary_key=True)
    org_id = Column(String, ForeignKey("orgs.id"), nullable=False)


class User(Base):
    __tablename__ = "users"

    id = Column(String(), primary_key=True)


class RepoRole(Base):
    __tablename__ = "repo_roles"
    id = Column(Integer, primary_key=True)
    user_id = Column(String, ForeignKey("users.id"), nullable=False)
    repo_id = Column(String, ForeignKey("repos.id"), nullable=False)
    user = relationship("User", backref="repo_roles", lazy=True)
    name = Column(String, index=True)


class OrgRole(Base):
    __tablename__ = "org_roles"
    id = Column(Integer, primary_key=True)
    user_id = Column(String, ForeignKey("users.id"), nullable=False)
    org_id = Column(String, ForeignKey("orgs.id"), nullable=False)
    user = relationship("User", backref="org_roles", lazy=True)
    name = Column(String, index=True)


engine = create_engine("sqlite:///:memory:")

Session = sessionmaker(bind=engine)
session = Session()

Base.metadata.create_all(engine)

# Here's some more test data
osohq = Organization(id="osohq")
apple = Organization(id="apple")

ios = Repository(id="ios", org_id="apple")
oso_repo = Repository(id="oso", org_id="osohq")
demo_repo = Repository(id="demo", org_id="osohq")

leina = User(id="leina")
steve = User(id="steve")

role_1 = OrgRole(user_id="leina", org_id="osohq", name="owner")

objs = {
    "leina": leina,
    "steve": steve,
    "osohq": osohq,
    "apple": apple,
    "ios": ios,
    "oso_repo": oso_repo,
    "demo_repo": demo_repo,
    "role_1": role_1,
}
for obj in objs.values():
    session.add(obj)
session.commit()
# docs: end-b1

# docs: begin-b2
# The query functions are the same.
def build_query_cls(cls):
    handlers = {
        'Eq': lambda a, b: a == b,
        'Neq': lambda a, b: a != b,
        'In': lambda a, b: a.in_(b),
        'Nin': lambda a, b: not_(a.in_(b)),
    }
    def build_query(filters):
        query = session.query(cls)
        for filter in filters:
            assert filter.kind in ["Eq", "In"]
            if filter.field is None:
                field = cls.id
                if fil.kind != 'Nin':
                    value = fil.value.id
                else:
                    value = [value.id for value in fil.value]
            elif isinstance(filter.field, list):
                field = [cls.id if fld is None else getattr(cls, fld)]
                value = filter.value
            else:
                field = getattr(cls, filter.field)
                value = filter.value

            if not isinstance(field, list):
                cond = handlers[filter.kind](field, value)
            else:
                combine = handlers['Eq' if filter.kind == 'In' else 'Neq']
                conds = [and_(*[co(*fv) for fv in zip(field, val)]) for val in value]
                cond = or_(*conds) if conds else false()

            query = query.filter(cond)
        return query
    return build_query


def exec_query(query):
    return query.all()


def combine_query(q1, q2):
    return q1.union(q2)


from oso import Oso, Relation

oso = Oso()

# All the combine/exec query functions are the same, so we
# can set defaults.
oso.set_data_filtering_query_defaults(
    exec_query=exec_query, combine_query=combine_query
)

oso.register_class(
    Organization,
    fields={
        "id": str,
    },
    build_query=build_query_cls(Organization),
)

oso.register_class(
    Repository,
    fields={
        "id": str,
        # Here we use a Relation to represent the logical connection between an Organization and a Repository.
        # Note that this only goes in one direction: to access repositories from an organization, we'd have to
        # add a "many" relation on the Organization class.
        "organization": Relation(
            kind="one", other_type="Organization", my_field="org_id", other_field="id"
        ),
    },
    build_query=build_query_cls(Repository),
)

oso.register_class(User, fields={"id": str, "repo_roles": list})
# docs: end-b2

with open("policy_b.polar") as f:
    policy_a = f.read()

# docs: begin-b3
oso.load_str(policy_a)
leina_repos = list(oso.authorized_resources(leina, "read", Repository))
assert leina_repos == [oso_repo, demo_repo]
# docs: end-b3

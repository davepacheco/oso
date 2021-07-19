import pytest

from pathlib import Path
from oso import Oso, OsoError, Variable
from polar import Expression, Pattern
from polar.exceptions import UnsupportedError


class User:
    def __init__(self, teams):
        self.teams = teams

    def has_role(self, role, resource):
        return True


class Team:
    def has_role(self, role, resource):
        return True


class Org:
    pass


class Repo:
    def __init__(self, org):
        self.org = org


def test_rebac_validation():
    o = Oso()
    o.register_actor(User, methods=["has_role"], properties=["teams"])
    o.register_group(Team, methods=["has_role"])
    o.register_resource(Org)
    o.register_resource(Repo, properties=["org"])
    o.load_file(Path(__file__).parent / "rebac_poc.polar")
    results = []
    try:
        results = list(
            o.query_rule(
                "has_role",
                Variable("actor"),
                Variable("role"),
                Variable("resource"),
                accept_expression=True,
                method_constraints=True,
            )
        )
    except UnsupportedError as e:
        if e.message.startswith("Not supported: cannot call method"):
            method = e.message.replace("Not supported: cannot call method ", "").split(
                " "
            )[0]
        print(e)

    for res in results:
        b = res["bindings"]
        actor = b["actor"]
        # actor type should always be abstract
        assert type(actor) == Expression
        assert get_specializer_tag(actor) in o.host.actors.keys()

        role = b["role"]
        assert type(role) == str

        resource = b["resource"]
        # resource type should always be abstract
        assert type(resource) == Expression
        assert get_specializer_tag(resource) in o.host.resources.keys()


def get_specializer_tag(expr):
    if expr.operator == "And" and len(expr.args) == 1:
        expr = expr.args[0]
    assert expr.operator == "Isa"
    spec = expr.args[1]
    assert type(spec) == Pattern
    return spec.tag
